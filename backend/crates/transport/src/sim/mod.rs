//! 결정론적 시뮬레이션 전송 (개념: sim). DST 하네스의 네트워크 절반 (D25).
//!
//! `NodeTransport`를 구현하되 **가상 시간**에 메시지를 스케줄하고, 시드 PRNG로 카오스
//! (지연·유실·파티션)를 주입한다. `send`는 즉시 큐에 적재만(실제 await 없음) → 하네스가
//! `advance_to`로 가상 시계를 진행시키면 그 시점까지 도착할 메시지를 노드별 ready 큐로 옮긴다.
//! 동일 시드 + 동일 send 순서 → **완전히 동일한 배달 순서/유실** (재현 가능, D25).
//!
//! 시간 절반(SimClock)은 `node::ManualClock`을 그대로 쓴다 — 같은 now_ms를 공유.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use protocol::NodeMessage;

use crate::iface::{Inbound, NodeTransport, TransportError};

/// 결정론적 PRNG (splitmix64). 시드 동일 → 동일 수열.
pub struct DetRng(u64);

impl DetRng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// [0,1) 실수.
    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    /// [lo, hi] 정수 (양끝 포함).
    fn range(&mut self, lo: u64, hi: u64) -> u64 {
        if hi <= lo { lo } else { lo + self.next_u64() % (hi - lo + 1) }
    }
}

/// 카오스 설정. 기본 = 무카오스(지연 0, 유실 0).
#[derive(Clone)]
pub struct SimConfig {
    pub min_latency_ms: u64,
    pub max_latency_ms: u64,
    /// 메시지 유실 확률 0.0..=1.0.
    pub drop_prob: f64,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self { min_latency_ms: 0, max_latency_ms: 0, drop_prob: 0.0 }
    }
}

struct Scheduled {
    at: u64,
    seq: u64,
    dst: u64,
    inb: Inbound,
}
// (at, seq) 오름차순 정렬용 — BinaryHeap은 max-heap이라 Reverse로 min-heap.
impl PartialEq for Scheduled {
    fn eq(&self, o: &Self) -> bool {
        (self.at, self.seq) == (o.at, o.seq)
    }
}
impl Eq for Scheduled {}
impl PartialOrd for Scheduled {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Scheduled {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        (self.at, self.seq).cmp(&(o.at, o.seq))
    }
}

struct Net {
    now: u64,
    seq: u64,
    rng: DetRng,
    cfg: SimConfig,
    nodes: HashSet<u64>,
    isolated: HashSet<u64>,
    scheduled: BinaryHeap<Reverse<Scheduled>>,
    ready: HashMap<u64, VecDeque<Inbound>>,
    dropped: u64,
}

/// 결정론적 시뮬레이션 네트워크. 노드별 `SimTransport`를 발급하고, 하네스가 시간을 진행시킨다.
#[derive(Clone)]
pub struct SimNetwork {
    inner: Arc<Mutex<Net>>,
}

impl SimNetwork {
    pub fn new(seed: u64, cfg: SimConfig) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Net {
                now: 0,
                seq: 0,
                rng: DetRng::new(seed),
                cfg,
                nodes: HashSet::new(),
                isolated: HashSet::new(),
                scheduled: BinaryHeap::new(),
                ready: HashMap::new(),
                dropped: 0,
            })),
        }
    }

    /// 노드 등록 → 그 노드의 전송 핸들.
    pub fn transport(&self, node_id: u64) -> SimTransport {
        let mut net = self.inner.lock().unwrap();
        net.nodes.insert(node_id);
        net.ready.entry(node_id).or_default();
        SimTransport { node_id, net: self.clone() }
    }

    pub fn now(&self) -> u64 {
        self.inner.lock().unwrap().now
    }

    /// 가상 시계를 `t`까지 진행 → 그 시점까지 도착 예정인 메시지를 (at,seq) 순으로 ready 큐에 배달.
    pub fn advance_to(&self, t: u64) {
        let mut net = self.inner.lock().unwrap();
        net.now = net.now.max(t);
        let now = net.now;
        while let Some(Reverse(top)) = net.scheduled.peek() {
            if top.at > now {
                break;
            }
            let Reverse(s) = net.scheduled.pop().unwrap();
            net.ready.entry(s.dst).or_default().push_back(s.inb);
        }
    }

    /// `dt`만큼 진행.
    pub fn advance(&self, dt: u64) {
        let target = self.now() + dt;
        self.advance_to(target);
    }

    /// 아직 배달 대기 중인 인플라이트 메시지의 가장 이른 도착 시각(스텝 루프용).
    pub fn next_event_time(&self) -> Option<u64> {
        self.inner.lock().unwrap().scheduled.peek().map(|Reverse(s)| s.at)
    }

    /// 노드의 ready 큐를 비워 반환 (하네스가 `router.handle_inbound`로 처리).
    pub fn take_inbound(&self, node_id: u64) -> Vec<Inbound> {
        let mut net = self.inner.lock().unwrap();
        net.ready.get_mut(&node_id).map(|q| q.drain(..).collect()).unwrap_or_default()
    }

    /// 노드 격리(파티션) — 이후 그 노드의 송수신 메시지는 유실.
    pub fn partition(&self, node_id: u64) {
        self.inner.lock().unwrap().isolated.insert(node_id);
    }
    /// 파티션 해제.
    pub fn heal(&self, node_id: u64) {
        self.inner.lock().unwrap().isolated.remove(&node_id);
    }

    /// 누적 유실 메시지 수 (드롭+파티션).
    pub fn dropped(&self) -> u64 {
        self.inner.lock().unwrap().dropped
    }
}

/// 한 노드의 시뮬레이션 전송 핸들.
#[derive(Clone)]
pub struct SimTransport {
    node_id: u64,
    net: SimNetwork,
}

impl NodeTransport for SimTransport {
    fn local_node_id(&self) -> u64 {
        self.node_id
    }

    async fn send(&self, dest: u64, msg: NodeMessage) -> Result<(), TransportError> {
        let mut net = self.net.inner.lock().unwrap();
        if !net.nodes.contains(&dest) {
            return Err(TransportError::UnknownNode(dest));
        }
        // 파티션: 송신/수신 어느 쪽이든 격리면 유실 (fire-and-forget이라 Ok 반환).
        if net.isolated.contains(&self.node_id) || net.isolated.contains(&dest) {
            net.dropped += 1;
            return Ok(());
        }
        // 유실 확률 (시드 RNG).
        if net.rng.unit() < net.cfg.drop_prob {
            net.dropped += 1;
            return Ok(());
        }
        let (lo, hi) = (net.cfg.min_latency_ms, net.cfg.max_latency_ms);
        let delay = net.rng.range(lo, hi);
        let at = net.now + delay;
        let seq = net.seq;
        net.seq += 1;
        net.scheduled.push(Reverse(Scheduled { at, seq, dst: dest, inb: Inbound { src: self.node_id, msg } }));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(min: u64, max: u64, drop: f64) -> SimConfig {
        SimConfig { min_latency_ms: min, max_latency_ms: max, drop_prob: drop }
    }

    /// 지연: 도착 시각 전엔 ready에 없고, 시계를 진행시키면 배달된다.
    #[tokio::test]
    async fn latency_holds_message_until_clock_advances() {
        let net = SimNetwork::new(1, cfg(5, 5, 0.0));
        let t1 = net.transport(1);
        let _t2 = net.transport(2);
        t1.send(2, NodeMessage::Ping).await.unwrap();

        net.advance_to(4);
        assert!(net.take_inbound(2).is_empty(), "도착 전");
        net.advance_to(5);
        let got = net.take_inbound(2);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].src, 1);
    }

    /// 동일 시드 + 동일 송신 순서 → 동일 배달 순서 (재현성).
    #[tokio::test]
    async fn same_seed_same_order() {
        let run = || async {
            let net = SimNetwork::new(42, cfg(1, 20, 0.0));
            let t1 = net.transport(1);
            let _t2 = net.transport(2);
            for i in 0..20u64 {
                t1.send(2, NodeMessage::RealmFanout {
                    realm_id: i, t: "MESSAGE_CREATE".into(), payload: i.to_string(), user_ids: vec![],
                }).await.unwrap();
            }
            net.advance_to(1000);
            net.take_inbound(2)
                .into_iter()
                .map(|ib| match ib.msg {
                    NodeMessage::RealmFanout { payload, .. } => payload,
                    _ => String::new(),
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(run().await, run().await);
    }

    /// 유실 확률 1.0 → 메시지가 배달되지 않음.
    #[tokio::test]
    async fn full_drop_loses_everything() {
        let net = SimNetwork::new(7, cfg(1, 1, 1.0));
        let t1 = net.transport(1);
        let _t2 = net.transport(2);
        t1.send(2, NodeMessage::Ping).await.unwrap();
        net.advance_to(100);
        assert!(net.take_inbound(2).is_empty());
        assert_eq!(net.dropped(), 1);
    }

    /// 파티션된 노드로는 배달되지 않고, 해제하면 다시 배달된다.
    #[tokio::test]
    async fn partition_isolates_node() {
        let net = SimNetwork::new(3, cfg(0, 0, 0.0));
        let t1 = net.transport(1);
        let _t2 = net.transport(2);

        net.partition(2);
        t1.send(2, NodeMessage::Ping).await.unwrap();
        net.advance_to(10);
        assert!(net.take_inbound(2).is_empty(), "파티션 중 유실");

        net.heal(2);
        t1.send(2, NodeMessage::Ping).await.unwrap();
        net.advance_to(20);
        assert_eq!(net.take_inbound(2).len(), 1, "해제 후 배달");
    }

    #[tokio::test]
    async fn unknown_node_errors() {
        let net = SimNetwork::new(1, SimConfig::default());
        let t1 = net.transport(1);
        assert_eq!(t1.send(99, NodeMessage::Ping).await, Err(TransportError::UnknownNode(99)));
    }
}
