//! 2단 라우팅 + 크로스노드 팬아웃 (개념: router). ring + transport + 로컬 Realm 액터 (D9/D12/D24).
//!
//! 세션 소유(클라가 붙은 노드) vs Realm 소유(hash(realm)) 분리:
//! - Realm 로컬 소유 → 로컬 액터로 디스패치
//! - 원격 소유 → transport로 소유 노드에 포워딩(RealmSend/Subscribe)
//! - 소유 노드는 액터 이벤트를 받아 구독자 노드들로 RealmFanout 전파
//!
//! 흐름: `route_send` → (소유 노드) Realm 액터 → events → `fanout` → 로컬배달 + 원격 RealmFanout
//!        원격 노드: `handle_inbound(RealmFanout)` → 로컬 세션 배달(LocalDelivery)

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use actor_rt::{Mailbox, spawn};
use domain::id::{ChannelId, MessageId, RealmId, Snowflake, SnowflakeGenerator, UserId};
use protocol::NodeMessage;
use tokio::sync::{mpsc, oneshot};
use transport::{Inbound, NodeTransport, TransportError};

use crate::clock::Clock;
use crate::membership::Membership;
use crate::realm::{RealmActor, RealmCommand, RealmEvent};
use crate::ring::HashRing;

#[derive(Debug, thiserror::Error)]
pub enum RouterError {
    #[error("no owner node for realm (empty ring)")]
    NoOwner,
    #[error("realm actor unavailable")]
    ActorGone,
    #[error(transparent)]
    Transport(#[from] TransportError),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Routed {
    Local,
    Forwarded { to: u64 },
}

/// 이 노드의 로컬 세션으로 배달할 메시지 (gateway가 WS로 push, 후속).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalDelivery {
    pub realm: RealmId,
    pub channel_id: ChannelId,
    pub message_id: MessageId,
    pub author: UserId,
    pub content: String,
    /// 클라 멱등성 키 — 작성자 세션 에코용 (D34).
    pub nonce: Option<String>,
    /// 이 노드의 로컬 대상 유저들.
    pub user_ids: Vec<u64>,
}

pub struct Router<T: NodeTransport> {
    local_node_id: u64,
    /// 노드당 단일 Snowflake generator — 모든 Realm 액터에 주입 (D11 불변식).
    snowflakes: Arc<SnowflakeGenerator>,
    ring: HashRing,
    /// 피어 생사 뷰 — down 노드는 소유권 탐색에서 제외(자동 failover, D23).
    membership: Arc<Membership>,
    /// 주입된 시계 — Realm 액터에 전달. DST에선 SimClock(ManualClock)로 결정론 (D25).
    clock: Arc<dyn Clock>,
    transport: T,
    events: mpsc::Sender<RealmEvent>,
    local_realms: Mutex<HashMap<u64, Mailbox<RealmCommand>>>,
}

impl<T: NodeTransport> Router<T> {
    /// `snowflakes` = 노드당 단일 generator (server가 소유, Router·REST 등에 공유 주입). D11.
    /// `clock` = 노드 시계(D25) — Realm 액터에 주입. 단일노드는 SystemClock, DST는 SimClock.
    pub fn new(
        local_node_id: u64,
        snowflakes: Arc<SnowflakeGenerator>,
        clock: Arc<dyn Clock>,
        ring: HashRing,
        transport: T,
        events: mpsc::Sender<RealmEvent>,
    ) -> Self {
        Self {
            local_node_id,
            snowflakes,
            ring,
            membership: Arc::new(Membership::new()),
            clock,
            transport,
            events,
            local_realms: Mutex::new(HashMap::new()),
        }
    }

    /// 공유 생사 뷰 — server의 failure detector 루프가 갱신, owner가 소비 (D23).
    pub fn membership(&self) -> &Arc<Membership> {
        &self.membership
    }

    /// Realm 소유 노드 — down 노드(D23)를 건너뛴 일관 해싱 소유권.
    pub fn owner(&self, realm: RealmId) -> Option<u64> {
        self.ring.owner_excluding(realm.0.raw(), &self.membership.down_set())
    }

    pub fn is_local(&self, realm: RealmId) -> bool {
        self.owner(realm) == Some(self.local_node_id)
    }

    fn local_realm(&self, realm: RealmId) -> Mailbox<RealmCommand> {
        let mut map = self.local_realms.lock().unwrap();
        map.entry(realm.0.raw())
            .or_insert_with(|| {
                let actor = RealmActor::new(
                    realm,
                    Arc::clone(&self.snowflakes),
                    Arc::clone(&self.clock),
                    self.events.clone(),
                );
                spawn(actor, 256)
            })
            .clone()
    }

    /// 구독 라우팅 (D12).
    pub async fn route_subscribe(
        &self,
        realm: RealmId,
        user: UserId,
        user_node: u64,
    ) -> Result<Routed, RouterError> {
        match self.owner(realm) {
            None => Err(RouterError::NoOwner),
            Some(o) if o == self.local_node_id => {
                self.local_realm(realm)
                    .send(RealmCommand::Subscribe { user, node: user_node })
                    .await
                    .map_err(|_| RouterError::ActorGone)?;
                Ok(Routed::Local)
            }
            Some(o) => {
                self.transport
                    .send(
                        o,
                        NodeMessage::Subscribe {
                            realm_id: realm.0.raw(),
                            user_id: user.0.raw(),
                            node_id: user_node,
                        },
                    )
                    .await?;
                Ok(Routed::Forwarded { to: o })
            }
        }
    }

    /// 메시지 전송 (fire-and-forget). 로컬/원격 모두. 결과는 events→fanout 경로로 흐름.
    pub async fn route_send(
        &self,
        realm: RealmId,
        channel: ChannelId,
        author: UserId,
        content: String,
        nonce: Option<String>,
    ) -> Result<Routed, RouterError> {
        match self.owner(realm) {
            None => Err(RouterError::NoOwner),
            Some(o) if o == self.local_node_id => {
                let (tx, _rx) = oneshot::channel();
                self.local_realm(realm)
                    .send(RealmCommand::SendMessage {
                        channel_id: channel,
                        author,
                        content,
                        nonce,
                        reply: tx,
                    })
                    .await
                    .map_err(|_| RouterError::ActorGone)?;
                Ok(Routed::Local)
            }
            Some(o) => {
                self.transport
                    .send(
                        o,
                        NodeMessage::RealmSend {
                            realm_id: realm.0.raw(),
                            channel_id: channel.0.raw(),
                            author: author.0.raw(),
                            content,
                            nonce,
                        },
                    )
                    .await?;
                Ok(Routed::Forwarded { to: o })
            }
        }
    }

    /// 로컬 소유 Realm에 직접 전송하고 MessageId 회신 (테스트/단일노드 편의).
    pub async fn route_send_local(
        &self,
        realm: RealmId,
        channel: ChannelId,
        author: UserId,
        content: String,
        nonce: Option<String>,
    ) -> Result<MessageId, RouterError> {
        let (tx, rx) = oneshot::channel();
        self.local_realm(realm)
            .send(RealmCommand::SendMessage { channel_id: channel, author, content, nonce, reply: tx })
            .await
            .map_err(|_| RouterError::ActorGone)?;
        rx.await.map_err(|_| RouterError::ActorGone)
    }

    /// Realm 액터가 방출한 이벤트를 팬아웃 (소유 노드에서 실행).
    /// 노드별로 그룹화 → 로컬 대상은 `LocalDelivery`로 반환, 원격 노드엔 `RealmFanout` 전송.
    pub async fn fanout(&self, event: RealmEvent) -> Result<Vec<LocalDelivery>, RouterError> {
        let RealmEvent::MessageCreated { realm, channel_id, message_id, author, content, nonce, targets } =
            event;

        let mut by_node: HashMap<u64, Vec<u64>> = HashMap::new();
        for (user, node) in targets {
            by_node.entry(node).or_default().push(user.0.raw());
        }

        let mut local = Vec::new();
        for (node, user_ids) in by_node {
            if node == self.local_node_id {
                local.push(LocalDelivery {
                    realm,
                    channel_id,
                    message_id,
                    author,
                    content: content.clone(),
                    nonce: nonce.clone(),
                    user_ids,
                });
            } else {
                self.transport
                    .send(
                        node,
                        NodeMessage::RealmFanout {
                            realm_id: realm.0.raw(),
                            channel_id: channel_id.0.raw(),
                            message_id: message_id.0.raw(),
                            author: author.0.raw(),
                            content: content.clone(),
                            nonce: nonce.clone(),
                            user_ids,
                        },
                    )
                    .await?;
            }
        }
        Ok(local)
    }

    /// 수신한 노드 메시지 처리. RealmFanout이면 로컬 배달 반환.
    pub async fn handle_inbound(
        &self,
        inbound: Inbound,
    ) -> Result<Option<LocalDelivery>, RouterError> {
        match inbound.msg {
            NodeMessage::Subscribe { realm_id, user_id, node_id } => {
                self.local_realm(RealmId(Snowflake::from_raw(realm_id)))
                    .send(RealmCommand::Subscribe {
                        user: UserId(Snowflake::from_raw(user_id)),
                        node: node_id,
                    })
                    .await
                    .map_err(|_| RouterError::ActorGone)?;
                Ok(None)
            }
            NodeMessage::Unsubscribe { realm_id, user_id, .. } => {
                self.local_realm(RealmId(Snowflake::from_raw(realm_id)))
                    .send(RealmCommand::Unsubscribe { user: UserId(Snowflake::from_raw(user_id)) })
                    .await
                    .map_err(|_| RouterError::ActorGone)?;
                Ok(None)
            }
            NodeMessage::RealmSend { realm_id, channel_id, author, content, nonce } => {
                let (tx, _rx) = oneshot::channel();
                self.local_realm(RealmId(Snowflake::from_raw(realm_id)))
                    .send(RealmCommand::SendMessage {
                        channel_id: ChannelId(Snowflake::from_raw(channel_id)),
                        author: UserId(Snowflake::from_raw(author)),
                        content,
                        nonce,
                        reply: tx,
                    })
                    .await
                    .map_err(|_| RouterError::ActorGone)?;
                Ok(None)
            }
            NodeMessage::RealmFanout {
                realm_id,
                channel_id,
                message_id,
                author,
                content,
                nonce,
                user_ids,
            } => Ok(Some(LocalDelivery {
                realm: RealmId(Snowflake::from_raw(realm_id)),
                channel_id: ChannelId(Snowflake::from_raw(channel_id)),
                message_id: MessageId(Snowflake::from_raw(message_id)),
                author: UserId(Snowflake::from_raw(author)),
                content,
                nonce,
                user_ids,
            })),
            // 생사 판정 (D23): PING 받으면 PONG 회신. 수신 시각 기록은 server inbound 루프가
            // 주입된 clock으로 수행(Router는 clock-free 유지). PONG/HELLO는 수신 자체가 liveness.
            NodeMessage::Ping => {
                self.transport.send(inbound.src, NodeMessage::Pong).await?;
                Ok(None)
            }
            NodeMessage::Pong | NodeMessage::Hello { .. } | NodeMessage::HelloAck { .. } => Ok(None),
        }
    }
}

/// PING/PONG failure detector 루프 (D23). server가 `tokio::spawn`.
/// 주기적으로 각 피어에 PING 송신 + `Membership::sweep`로 timeout 초과 피어를 down 처리.
/// PONG/임의 트래픽 수신 시각은 server inbound 루프가 `record_seen`으로 갱신.
pub async fn run_failure_detector<T: NodeTransport>(
    transport: T,
    membership: Arc<Membership>,
    peers: Vec<u64>,
    clock: Arc<dyn Clock>,
    interval_ms: u64,
    timeout_ms: u64,
) {
    // 시작 시 grace: 아직 PONG 전이라도 즉시 down 판정되지 않도록 seed.
    let now = clock.now_ms();
    for &p in &peers {
        membership.record_seen(p, now);
    }
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(interval_ms));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        for &p in &peers {
            let _ = transport.send(p, NodeMessage::Ping).await; // 미연결이면 무시(이후 sweep이 down).
        }
        membership.sweep(&peers, clock.now_ms(), timeout_ms);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use transport::Switchboard;

    fn uid(n: u64) -> UserId {
        UserId(Snowflake::from_raw(n))
    }
    fn mkgen(worker: u16) -> Arc<SnowflakeGenerator> {
        Arc::new(SnowflakeGenerator::new(worker))
    }
    fn clk() -> Arc<dyn Clock> {
        Arc::new(crate::clock::SystemClock)
    }
    fn ring_2() -> HashRing {
        let mut r = HashRing::new(100);
        r.add_node(1);
        r.add_node(2);
        r
    }
    fn first_realm_owned_by(ring: &HashRing, node: u64) -> RealmId {
        (0u64..)
            .map(|x| RealmId(Snowflake::from_raw(x)))
            .find(|r| ring.owner(r.0.raw()) == Some(node))
            .unwrap()
    }

    #[tokio::test]
    async fn subscribe_routes_local_vs_remote() {
        let board = Switchboard::new();
        let (t1, _r1) = board.join(1, 16);
        let (_t2, mut r2) = board.join(2, 16);

        let ring = ring_2();
        let local_realm = first_realm_owned_by(&ring, 1);
        let remote_realm = first_realm_owned_by(&ring, 2);

        let (etx, _erx) = mpsc::channel(64);
        let router = Router::new(1, mkgen(1), clk(), ring, t1, etx);

        assert_eq!(router.route_subscribe(local_realm, uid(0xA), 1).await.unwrap(), Routed::Local);
        assert_eq!(
            router.route_subscribe(remote_realm, uid(0xB), 1).await.unwrap(),
            Routed::Forwarded { to: 2 }
        );
        let inbound = r2.recv().await.unwrap();
        assert_eq!(inbound.src, 1);
        assert!(matches!(inbound.msg, NodeMessage::Subscribe { node_id: 1, .. }));
    }

    #[tokio::test]
    async fn local_send_flows_through_actor_to_event() {
        let board = Switchboard::new();
        let (t1, _r1) = board.join(1, 16);
        let mut ring = HashRing::new(100);
        ring.add_node(1);

        let (etx, mut erx) = mpsc::channel(64);
        let router = Router::new(1, mkgen(1), clk(), ring, t1, etx);
        let realm = RealmId(Snowflake::from_raw(42));

        router.route_subscribe(realm, uid(1), 1).await.unwrap();
        let chan = ChannelId(Snowflake::from_raw(0xC0));
        let mid = router.route_send_local(realm, chan, uid(1), "hello".into(), None).await.unwrap();

        let RealmEvent::MessageCreated { message_id, content, targets, .. } =
            erx.recv().await.unwrap();
        assert_eq!(message_id, mid);
        assert_eq!(content, "hello");
        assert_eq!(targets.len(), 1);
    }

    /// 분산 메시지 경로의 정점: 두 노드에 흩어진 구독자에게 한 메시지가 팬아웃되는 종단 흐름.
    #[tokio::test]
    async fn cross_node_fanout_end_to_end() {
        let board = Switchboard::new();
        let (t1, mut r1) = board.join(1, 64);
        let (t2, mut r2) = board.join(2, 64);

        let realm = first_realm_owned_by(&ring_2(), 1); // 노드1 소유

        let (etx1, mut erx1) = mpsc::channel(64);
        let (etx2, _erx2) = mpsc::channel(64);
        let router1 = Router::new(1, mkgen(1), clk(), ring_2(), t1, etx1);
        let router2 = Router::new(2, mkgen(2), clk(), ring_2(), t2, etx2);

        // A는 노드1(소유)에서 구독 — 로컬
        router1.route_subscribe(realm, uid(0xA), 1).await.unwrap();
        // B는 노드2에서 구독 → 소유 노드1로 포워딩
        assert_eq!(
            router2.route_subscribe(realm, uid(0xB), 2).await.unwrap(),
            Routed::Forwarded { to: 1 }
        );
        // 노드1이 포워딩된 Subscribe 처리 (B@node2 등록)
        let sub = r1.recv().await.unwrap();
        assert!(router1.handle_inbound(sub).await.unwrap().is_none());

        // A가 노드1에서 메시지 전송
        let chan = ChannelId(Snowflake::from_raw(0xC0));
        router1.route_send(realm, chan, uid(0xA), "hi".into(), None).await.unwrap();

        // 노드1: 액터 이벤트 수신 → 팬아웃
        let event = erx1.recv().await.unwrap();
        let local = router1.fanout(event).await.unwrap();
        assert_eq!(local.len(), 1);
        assert_eq!(local[0].user_ids, vec![0xA]); // A는 노드1 로컬 배달

        // 노드2: RealmFanout 수신 → B 로컬 배달
        let fan = r2.recv().await.unwrap();
        let delivery = router2.handle_inbound(fan).await.unwrap().unwrap();
        assert_eq!(delivery.user_ids, vec![0xB]);
        assert_eq!(delivery.content, "hi");
    }

    /// 생사 판정 → 소유권 failover (D23): 소유 노드가 down으로 표시되면 다음 살아있는 노드가
    /// 그 Realm을 소유 → 새 소유 노드에서 로컬 전송이 동작(액터 fresh-spawn = rehydrate).
    #[tokio::test]
    async fn owner_fails_over_when_node_marked_down() {
        let board = Switchboard::new();
        let (t2, _r2) = board.join(2, 16);
        let ring = ring_2();
        let realm = first_realm_owned_by(&ring, 1); // 평소 노드1 소유

        let (etx, mut erx) = mpsc::channel(64);
        // 노드2 관점의 Router.
        let router2 = Router::new(2, mkgen(2), clk(), ring, t2, etx);
        assert_eq!(router2.owner(realm), Some(1));
        assert!(!router2.is_local(realm));

        // 노드1 down 판정 → 소유권이 노드2로 이동.
        router2.membership().mark_down(1);
        assert_eq!(router2.owner(realm), Some(2));
        assert!(router2.is_local(realm), "failover 후 노드2가 소유");

        // 새 소유 노드에서 로컬 전송이 동작(액터가 새로 떠서 rehydrate).
        let chan = ChannelId(Snowflake::from_raw(0xC0));
        router2.route_subscribe(realm, uid(0xB), 2).await.unwrap();
        router2.route_send_local(realm, chan, uid(0xB), "after-failover".into(), None).await.unwrap();
        let RealmEvent::MessageCreated { content, .. } = erx.recv().await.unwrap();
        assert_eq!(content, "after-failover");

        // 노드1 복귀 → 소유권 환원(일관 해싱 re-join).
        router2.membership().record_seen(1, 1);
        assert_eq!(router2.owner(realm), Some(1));
    }

    /// Phase 2 종단: **실제 raw-TCP+mTLS 전송** 위에서 두 노드 크로스노드 팬아웃 (D3/D16).
    /// stub 대신 `TcpTransport`로 동일 시나리오 — 구독 포워딩 + RealmFanout 배달이 네트워크로 동작.
    #[tokio::test]
    async fn cross_node_fanout_over_tcp_mtls() {
        use std::sync::Arc;
        use std::time::Duration;
        use transport::{TcpTransport, client_config, generate_mesh, init_crypto, server_config};

        init_crypto();
        let mesh = generate_mesh(&["127.0.0.1", "127.0.0.1"]).unwrap();

        // 전송: 노드2 listen, 노드1 dial (작은 id가 큰 id에게, D4).
        let t1 = TcpTransport::new(1);
        let t2 = TcpTransport::new(2);
        let (in1_tx, mut in1_rx) = mpsc::channel(64);
        let (in2_tx, mut in2_rx) = mpsc::channel(64);
        let addr2 = t2
            .listen("127.0.0.1:0", server_config(&mesh.material(1)).unwrap(), in2_tx)
            .await
            .unwrap();
        t1.dial(2, addr2.to_string(), "127.0.0.1".into(), client_config(&mesh.material(0)).unwrap(), in1_tx);

        let (etx1, mut erx1) = mpsc::channel(64);
        let (etx2, _erx2) = mpsc::channel(64);
        let router1 = Arc::new(Router::new(1, mkgen(1), clk(), ring_2(), t1, etx1));
        let router2 = Arc::new(Router::new(2, mkgen(2), clk(), ring_2(), t2, etx2));
        let realm = first_realm_owned_by(&ring_2(), 1); // 노드1 소유

        // 노드1 inbound 루프: 포워딩된 Subscribe 처리.
        {
            let r = Arc::clone(&router1);
            tokio::spawn(async move {
                while let Some(ib) = in1_rx.recv().await {
                    let _ = r.handle_inbound(ib).await;
                }
            });
        }
        // 노드2 inbound 루프: RealmFanout → LocalDelivery 수집.
        let (deliv_tx, mut deliv_rx) = mpsc::channel(16);
        {
            let r = Arc::clone(&router2);
            tokio::spawn(async move {
                while let Some(ib) = in2_rx.recv().await {
                    if let Ok(Some(d)) = r.handle_inbound(ib).await {
                        let _ = deliv_tx.send(d).await;
                    }
                }
            });
        }

        // A는 노드1(소유) 로컬 구독.
        router1.route_subscribe(realm, uid(0xA), 1).await.unwrap();
        // B는 노드2에서 구독 → 소유 노드1로 네트워크 포워딩 (연결 수립까지 재시도).
        let mut forwarded = false;
        for _ in 0..40 {
            if matches!(router2.route_subscribe(realm, uid(0xB), 2).await, Ok(Routed::Forwarded { to: 1 })) {
                forwarded = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(forwarded, "구독 포워딩이 연결 수립 내 완료되지 않음");
        // 노드1이 포워딩된 Subscribe를 처리할 시간.
        tokio::time::sleep(Duration::from_millis(200)).await;

        // A가 노드1에서 전송 → 팬아웃.
        let chan = ChannelId(Snowflake::from_raw(0xC0));
        router1.route_send(realm, chan, uid(0xA), "hi-tcp".into(), None).await.unwrap();
        let event = erx1.recv().await.unwrap();
        let local = router1.fanout(event).await.unwrap();
        assert_eq!(local.iter().flat_map(|d| d.user_ids.clone()).collect::<Vec<_>>(), vec![0xA]);

        // 노드2: 네트워크로 RealmFanout 수신 → B 배달.
        let delivery = tokio::time::timeout(Duration::from_secs(2), deliv_rx.recv())
            .await
            .expect("배달 타임아웃")
            .expect("채널 종료");
        assert_eq!(delivery.user_ids, vec![0xB]);
        assert_eq!(delivery.content, "hi-tcp");
    }
}
