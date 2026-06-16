//! SWIM DST 하네스 (D45/D25) — SimNetwork + 가상 시간으로 동적 멤버십을 결정론 재현.
//!
//! 검증:
//! 1. **동적 합류**: seed만 아는 신규 노드가 SwimJoin → gossip 감염으로 전 노드가 서로를 학습(링 수렴).
//! 2. **장애감지**: 파티션된 노드는 direct+indirect 모두 실패 → Suspect → suspicion 타임아웃 → Dead,
//!    그 사망이 gossip으로 전파되어 살아있는 노드들의 링에서 제거된다.
//!
//! 상태머신(`Swim`)은 now_ms/rng 주입 순수 로직이라, SimNetwork가 가상 시간에 메시지를 스케줄하면
//! 송신 순서·도착이 결정론적이다(동일 시드 → 동일 수렴).

use std::collections::HashSet;

use node::swim::{MemberState, Swim, SwimAction, SwimConfig};
use transport::{NodeTransport, SimConfig, SimNetwork, SimTransport};

fn cfg() -> SwimConfig {
    SwimConfig {
        ping_timeout_ms: 100,
        probe_period_ms: 300,
        suspicion_timeout_ms: 500,
        indirect_k: 2,
        gossip_fanout: 3,
        dissemination_count: 8,
        max_piggyback: 16,
        anti_entropy_ticks: 5,
    }
}

/// 한 노드의 SWIM 상태 + 전송 + 링 멤버십(테스트 검증용 HashSet).
struct Node {
    id: u64,
    swim: Swim,
    tx: SimTransport,
    ring: HashSet<u64>,
}

impl Node {
    fn new(net: &SimNetwork, id: u64, seeds: &[(u64, &str)]) -> Self {
        let swim = Swim::new(id, format!("127.0.0.1:70{id:02}"), cfg(), 1000 + id);
        for (sid, addr) in seeds {
            swim.seed_member(*sid, *addr);
        }
        let mut ring: HashSet<u64> = seeds.iter().map(|(s, _)| *s).collect();
        ring.insert(id); // 자기 포함
        Node { id, swim, tx: net.transport(id), ring }
    }

    /// 액션 실행: Send→sim 전송, RingAdd/Remove→링 멤버십 갱신.
    async fn apply(&mut self, actions: Vec<SwimAction>) {
        for a in actions {
            match a {
                SwimAction::Send { to, msg } => {
                    let _ = self.tx.send(to, msg).await; // fire-and-forget(미등록/파티션은 유실)
                }
                SwimAction::RingAdd { node, .. } => {
                    self.ring.insert(node);
                }
                SwimAction::RingRemove { node } => {
                    self.ring.remove(&node);
                }
                SwimAction::Suspect { .. } => {}
            }
        }
    }
}

/// 가상 시간을 `interval`씩 `rounds`회 진행하며 각 노드 step + 도착 메시지 handle.
async fn drive(net: &SimNetwork, nodes: &mut [Node], rounds: u64, interval: u64) {
    for r in 1..=rounds {
        let now = r * interval;
        net.advance_to(now);
        // 1) 도착한 인바운드 처리(handle).
        for n in nodes.iter_mut() {
            for ib in net.take_inbound(n.id) {
                let actions = n.swim.handle(ib.src, &ib.msg, now);
                n.apply(actions).await;
            }
        }
        // 2) 주기 step.
        for n in nodes.iter_mut() {
            let actions = n.swim.step(now);
            n.apply(actions).await;
        }
    }
}

/// 노드1·2는 서로 알고, 노드3은 seed(1)만 알고 합류 → 전원이 서로 학습(링 수렴).
#[tokio::test]
async fn dynamic_join_converges() {
    let net = SimNetwork::new(7, SimConfig { min_latency_ms: 1, max_latency_ms: 5, drop_prob: 0.0 });
    let mut nodes = vec![
        Node::new(&net, 1, &[(2, "127.0.0.1:7002")]),
        Node::new(&net, 2, &[(1, "127.0.0.1:7001")]),
        Node::new(&net, 3, &[(1, "127.0.0.1:7001")]), // 신규: seed 1만 앎
    ];

    // 노드3 합류 공지: seed 1에게 SwimJoin.
    let join = nodes[2].swim.join_message();
    nodes[2].tx.send(1, join).await.unwrap();

    drive(&net, &mut nodes, 40, 100).await;

    // 전 노드가 1·2·3을 Alive로 학습 + 링에 포함.
    for n in &nodes {
        for other in [1u64, 2, 3] {
            if other == n.id {
                continue;
            }
            assert_eq!(
                n.swim.state_of(other),
                Some(MemberState::Alive),
                "노드{}가 노드{other}를 Alive로 알아야 함",
                n.id
            );
        }
        assert_eq!(n.ring, HashSet::from([1, 2, 3]), "노드{} 링 수렴", n.id);
    }
}

/// 동일 시드 → 동일 수렴(재현성, D25).
#[tokio::test]
async fn dynamic_join_deterministic() {
    async fn run() -> Vec<(u64, MemberState)> {
        let net = SimNetwork::new(99, SimConfig { min_latency_ms: 1, max_latency_ms: 8, drop_prob: 0.0 });
        let mut nodes = vec![
            Node::new(&net, 1, &[(2, "b")]),
            Node::new(&net, 2, &[(1, "a")]),
            Node::new(&net, 3, &[(1, "a")]),
        ];
        let join = nodes[2].swim.join_message();
        nodes[2].tx.send(1, join).await.unwrap();
        drive(&net, &mut nodes, 30, 100).await;
        let mut out: Vec<(u64, MemberState)> = Vec::new();
        for n in &nodes {
            for o in [1u64, 2, 3] {
                if let Some(s) = n.swim.state_of(o) {
                    out.push((n.id * 10 + o, s));
                }
            }
        }
        out
    }
    assert_eq!(run().await, run().await, "동일 시드는 동일 멤버십 수렴");
}

/// 파티션된 노드3 → 살아있는 1·2가 Suspect→Dead로 감지하고 링에서 제거(gossip 전파).
#[tokio::test]
async fn partitioned_node_detected_dead() {
    let net = SimNetwork::new(3, SimConfig { min_latency_ms: 1, max_latency_ms: 3, drop_prob: 0.0 });
    let mut nodes = vec![
        Node::new(&net, 1, &[(2, "b"), (3, "c")]),
        Node::new(&net, 2, &[(1, "a"), (3, "c")]),
        Node::new(&net, 3, &[(1, "a"), (2, "b")]),
    ];

    // 먼저 잠깐 정상 가동(서로 alive 확인).
    drive(&net, &mut nodes, 5, 100).await;
    // 노드3 격리 — 이후 송수신 유실.
    net.partition(3);

    // suspicion 타임아웃을 충분히 넘기도록 진행.
    drive(&net, &mut nodes, 40, 100).await;

    // 노드1·2는 노드3을 Dead로 감지 → 링에서 제거(Dead 전파 후 GC로 테이블에서 사라질 수 있음 = None).
    for id in [1u64, 2] {
        let n = nodes.iter().find(|n| n.id == id).unwrap();
        assert!(
            matches!(n.swim.state_of(3), Some(MemberState::Dead) | None),
            "노드{id}가 노드3을 Dead/제거로 감지 (현재 {:?})",
            n.swim.state_of(3)
        );
        assert!(!n.ring.contains(&3), "노드{id} 링에서 노드3 제거");
        // 자기 자신과 살아있는 동료는 링에 유지.
        let peer = if id == 1 { 2 } else { 1 };
        assert!(n.ring.contains(&id) && n.ring.contains(&peer), "노드{id} 링에 1·2 유지");
    }
}

/// 합류 후 즉시 같은 노드가 재합류(중복 SwimJoin)해도 멤버십이 일관 유지.
#[tokio::test]
async fn rejoin_is_idempotent() {
    let net = SimNetwork::new(11, SimConfig { min_latency_ms: 1, max_latency_ms: 2, drop_prob: 0.0 });
    let mut nodes =
        vec![Node::new(&net, 1, &[(2, "b")]), Node::new(&net, 2, &[(1, "a")])];
    // 노드2가 seed(1)에게 중복 SwimJoin을 3번 보냄.
    for _ in 0..3 {
        let join = nodes[1].swim.join_message();
        nodes[1].tx.send(1, join).await.ok();
    }
    drive(&net, &mut nodes, 20, 100).await;
    for n in &nodes {
        let other = if n.id == 1 { 2 } else { 1 };
        assert_eq!(n.swim.state_of(other), Some(MemberState::Alive));
    }
}
