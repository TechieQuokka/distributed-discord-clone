//! Realm 액터 (개념: realm). 한 Realm = 액터 1개 (D7).
//! 단일 소유 → Realm 내 메시지 순서 무료 보장(D24). 구독자표 보유(D12).
//!
//! 현재 컷: 인메모리. persist-then-fanout의 persist(Postgres)는 후속 배선(D24 TODO).

use std::collections::HashMap;
use std::sync::Arc;

use actor_rt::Actor;
use domain::id::{ChannelId, MessageId, RealmId, Snowflake, SnowflakeGenerator, UserId};
use tokio::sync::{mpsc, oneshot};

use crate::clock::Clock;

/// Realm 액터에 보내는 명령.
pub enum RealmCommand {
    /// 유저가 이 Realm을 구독(입장). `node` = 그 유저의 세션이 붙은 노드 (D12).
    Subscribe { user: UserId, node: u64 },
    Unsubscribe { user: UserId },
    /// 메시지 전송. 결과로 생성된 MessageId를 회신. 메시지는 채널 단위(D24).
    SendMessage {
        channel_id: ChannelId,
        author: UserId,
        content: String,
        nonce: Option<String>,
        /// 답장 대상 메시지 id (없으면 일반, D39).
        reference_message_id: Option<MessageId>,
        reply: oneshot::Sender<MessageId>,
    },
    /// 비-메시지 이벤트 팬아웃 (범용 envelope, D39). persist 없이 구독자표로 방출.
    /// `t`=DISPATCH 이벤트 이름, `payload`=직렬화된 JSON(불투명).
    Broadcast { t: String, payload: String },
}

/// Realm이 방출하는 이벤트(팬아웃 대상 포함).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RealmEvent {
    MessageCreated {
        realm: RealmId,
        channel_id: ChannelId,
        message_id: MessageId,
        author: UserId,
        content: String,
        /// 클라 멱등성 키 — 작성자 세션이 낙관적 전송과 대조 (D34).
        nonce: Option<String>,
        /// 답장 대상 메시지 id (D39).
        reference_message_id: Option<MessageId>,
        /// 팬아웃 대상 (user, node) — 구독자표 스냅샷 (D12).
        targets: Vec<(UserId, u64)>,
    },
    /// 비-메시지 이벤트 (범용 envelope, D39). dispatch 드라이버가 persist 없이 바로 팬아웃.
    Broadcast {
        realm: RealmId,
        t: String,
        payload: String,
        /// 팬아웃 대상 (user, node) — 구독자표 스냅샷 (D12).
        targets: Vec<(UserId, u64)>,
    },
}

pub struct RealmActor {
    realm_id: RealmId,
    /// 노드당 단일 generator를 주입받음 — 액터가 직접 만들지 않는다 (D11 불변식).
    snowflakes: Arc<SnowflakeGenerator>,
    /// 주입된 시계 — DST에선 SimClock(ManualClock)로 결정론 (D25/D11).
    clock: Arc<dyn Clock>,
    /// user(raw) → node. 팬아웃 위치추적 (D12).
    subscribers: HashMap<u64, u64>,
    events: mpsc::Sender<RealmEvent>,
}

impl RealmActor {
    pub fn new(
        realm_id: RealmId,
        snowflakes: Arc<SnowflakeGenerator>,
        clock: Arc<dyn Clock>,
        events: mpsc::Sender<RealmEvent>,
    ) -> Self {
        Self {
            realm_id,
            snowflakes,
            clock,
            subscribers: HashMap::new(),
            events,
        }
    }

    fn target_snapshot(&self) -> Vec<(UserId, u64)> {
        self.subscribers
            .iter()
            .map(|(&u, &n)| (UserId(Snowflake::from_raw(u)), n))
            .collect()
    }
}

impl Actor for RealmActor {
    type Message = RealmCommand;

    async fn handle(&mut self, msg: RealmCommand) {
        match msg {
            RealmCommand::Subscribe { user, node } => {
                self.subscribers.insert(user.0.raw(), node);
            }
            RealmCommand::Unsubscribe { user } => {
                self.subscribers.remove(&user.0.raw());
            }
            RealmCommand::SendMessage { channel_id, author, content, nonce, reference_message_id, reply } => {
                // 액터가 단일 소유자로서 ID·순서를 확정(D24). persist는 events 소비측(드라이버)이
                // 팬아웃 전에 수행(persist-then-fanout) — node 코어는 IO 무의존 유지(P2).
                let id = MessageId(self.snowflakes.next(self.clock.now_ms()));
                let event = RealmEvent::MessageCreated {
                    realm: self.realm_id,
                    channel_id,
                    message_id: id,
                    author,
                    content,
                    nonce,
                    reference_message_id,
                    targets: self.target_snapshot(),
                };
                let _ = self.events.send(event).await;
                let _ = reply.send(id);
            }
            RealmCommand::Broadcast { t, payload } => {
                // 비-메시지 이벤트: persist 없이 현재 구독자 스냅샷으로 방출 (D39).
                let event = RealmEvent::Broadcast {
                    realm: self.realm_id,
                    t,
                    payload,
                    targets: self.target_snapshot(),
                };
                let _ = self.events.send(event).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::ManualClock;
    use actor_rt::spawn;
    use domain::id::EPOCH_MS;

    fn uid(n: u64) -> UserId {
        UserId(Snowflake::from_raw(n))
    }
    fn realm(n: u64) -> RealmId {
        RealmId(Snowflake::from_raw(n))
    }
    fn mkgen(worker: u16) -> Arc<SnowflakeGenerator> {
        Arc::new(SnowflakeGenerator::new(worker))
    }

    #[tokio::test]
    async fn send_fans_out_and_ids_are_monotonic() {
        let (etx, mut erx) = mpsc::channel(16);
        let actor = RealmActor::new(realm(0x100), mkgen(1), Arc::new(ManualClock::new(EPOCH_MS + 1)), etx);
        let addr = spawn(actor, 16);

        addr.send(RealmCommand::Subscribe { user: uid(0xA), node: 7 }).await.unwrap();
        addr.send(RealmCommand::Subscribe { user: uid(0xB), node: 8 }).await.unwrap();

        let (r1, rx1) = oneshot::channel();
        addr.send(RealmCommand::SendMessage {
            channel_id: ChannelId(Snowflake::from_raw(0xC0)),
            author: uid(0xA),
            content: "hi".into(),
            nonce: None,
            reference_message_id: None,
            reply: r1,
        })
        .await
        .unwrap();
        let mid1 = rx1.await.unwrap();

        let (r2, rx2) = oneshot::channel();
        addr.send(RealmCommand::SendMessage {
            channel_id: ChannelId(Snowflake::from_raw(0xC0)),
            author: uid(0xA),
            content: "yo".into(),
            nonce: None,
            reference_message_id: None,
            reply: r2,
        })
        .await
        .unwrap();
        let mid2 = rx2.await.unwrap();

        assert!(mid2.0 > mid1.0, "single-owner actor must produce monotonic ids");

        let RealmEvent::MessageCreated { content, targets, .. } = erx.recv().await.unwrap() else {
            panic!("expected MessageCreated");
        };
        assert_eq!(content, "hi");
        assert_eq!(targets.len(), 2); // 구독자 2명 모두 팬아웃 대상
    }

    #[tokio::test]
    async fn unsubscribe_drops_from_targets() {
        let (etx, mut erx) = mpsc::channel(16);
        let addr = spawn(
            RealmActor::new(realm(1), mkgen(1), Arc::new(ManualClock::new(EPOCH_MS + 1)), etx),
            16,
        );
        addr.send(RealmCommand::Subscribe { user: uid(1), node: 1 }).await.unwrap();
        addr.send(RealmCommand::Unsubscribe { user: uid(1) }).await.unwrap();

        let (r, rx) = oneshot::channel();
        addr.send(RealmCommand::SendMessage {
            channel_id: ChannelId(Snowflake::from_raw(0xC0)),
            author: uid(1),
            content: "x".into(),
            nonce: None,
            reference_message_id: None,
            reply: r,
        })
        .await
        .unwrap();
        rx.await.unwrap();

        let RealmEvent::MessageCreated { targets, .. } = erx.recv().await.unwrap() else {
            panic!("expected MessageCreated");
        };
        assert_eq!(targets.len(), 0);
    }

    /// Broadcast(D39): 비-메시지 이벤트가 현재 구독자 스냅샷으로 방출된다(persist 무관).
    #[tokio::test]
    async fn broadcast_emits_with_subscriber_snapshot() {
        let (etx, mut erx) = mpsc::channel(16);
        let addr = spawn(
            RealmActor::new(realm(7), mkgen(1), Arc::new(ManualClock::new(EPOCH_MS + 1)), etx),
            16,
        );
        addr.send(RealmCommand::Subscribe { user: uid(0xA), node: 1 }).await.unwrap();
        addr.send(RealmCommand::Subscribe { user: uid(0xB), node: 2 }).await.unwrap();
        addr.send(RealmCommand::Broadcast {
            t: "GUILD_MEMBER_ADD".into(),
            payload: r#"{"x":1}"#.into(),
        })
        .await
        .unwrap();

        let RealmEvent::Broadcast { t, payload, targets, .. } = erx.recv().await.unwrap() else {
            panic!("expected Broadcast");
        };
        assert_eq!(t, "GUILD_MEMBER_ADD");
        assert_eq!(payload, r#"{"x":1}"#);
        assert_eq!(targets.len(), 2);
    }

    /// 회귀(D11): 같은 노드의 두 Realm이 **공유 generator**를 쓰면 같은 ms에도 ID가 유일.
    /// (과거 버그: Realm마다 generator를 따로 만들어 동일 ID 발급 가능했음.)
    #[tokio::test]
    async fn two_realms_sharing_generator_never_collide() {
        let shared = mkgen(1); // 노드당 1개
        let clock_ms = EPOCH_MS + 1;

        let mk = |r: u64| {
            let (etx, erx) = mpsc::channel(16);
            let addr = spawn(
                RealmActor::new(realm(r), Arc::clone(&shared), Arc::new(ManualClock::new(clock_ms)), etx),
                16,
            );
            (addr, erx)
        };
        let (a, mut arx) = mk(0xAAA);
        let (b, mut brx) = mk(0xBBB);

        let mut ids = std::collections::HashSet::new();
        for (addr, rx) in [(&a, &mut arx), (&b, &mut brx)] {
            for _ in 0..50 {
                let (tx, frx) = oneshot::channel();
                addr.send(RealmCommand::SendMessage {
            channel_id: ChannelId(Snowflake::from_raw(0xC0)),
            author: uid(1),
                    content: "x".into(),
                    nonce: None,
                    reference_message_id: None,
                    reply: tx,
                })
                .await
                .unwrap();
                let id = frx.await.unwrap();
                assert!(ids.insert(id.0.raw()), "두 Realm이 동일 ID 발급 — 유일성 위반");
                let _ = rx.recv().await.unwrap();
            }
        }
        assert_eq!(ids.len(), 100);
    }
}
