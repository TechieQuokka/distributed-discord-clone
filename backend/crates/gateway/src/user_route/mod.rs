//! 크로스노드 유저 이벤트 라우터 (개념: user_route). D43 — `UserEmitter`의 크로스노드 구현.
//!
//! D40/D41의 유저 단위 이벤트(`RELATIONSHIP_*`/`MESSAGE_ACK`)는 도입 당시 `Hub`(로컬 노드 세션)에만
//! 배달됐다. 여기서 대상 유저가 접속한 **모든 노드**로 일반화한다. `Presence` 디렉터리(D42,
//! user→호스팅 노드)를 라우팅 키로 **재사용**(새 레지스트리 0). presence처럼 풀메시 broadcast하지
//! 않고 호스팅 노드에만 **타깃 전송**한다(수신자가 특정 유저라 broadcast 불필요 — 상보적 패턴).
//!
//! - 로컬: `Hub::deliver` (detach 버퍼 세션도 포함 → RESUME 복구). 로컬 세션 없으면 자동 스킵.
//! - 원격: `Presence::nodes_for`로 호스팅 노드 조회 → `USER_DELIVER`(wire 0x0202) 타깃 전송.
//!
//! `RealmEmitter`=`Router`(D39)와 대칭 — server가 `Arc<dyn UserEmitter>`로 rest-api에 주입(Hub 대체).
//! 포트 시그니처(`emit_to_users`)는 불변이라 rest-api 라우트(relationship/read_state)는 무변경.

use std::collections::HashMap;
use std::sync::Arc;

use node::{Presence, Router};
use protocol::NodeMessage;
use serde_json::Value;
use transport::NodeTransport;

use crate::hub::Hub;
use crate::protocol::ServerEvent;

/// 유저 단위 이벤트의 크로스노드 emit 어댑터. Hub(로컬) + Presence(디렉터리) + Router(타깃 전송) 결합.
pub struct UserRouter<T: NodeTransport> {
    hub: Hub,
    presence: Arc<Presence>,
    router: Arc<Router<T>>,
    local_node_id: u64,
}

impl<T: NodeTransport> UserRouter<T> {
    pub fn new(hub: Hub, presence: Arc<Presence>, router: Arc<Router<T>>, local_node_id: u64) -> Self {
        Self { hub, presence, router, local_node_id }
    }
}

/// 수신한 `USER_DELIVER`(D43)를 이 노드의 로컬 세션에 배달. `dispatch::deliver_local`의 유저 버전 —
/// server inbound 루프가 `PRESENCE_GOSSIP`과 같은 자리에서 호출한다.
pub async fn deliver_user(hub: &Hub, t: String, payload: String, user_ids: &[u64]) {
    let d: Value = serde_json::from_str(&payload).unwrap_or(Value::Null);
    hub.deliver(user_ids, &ServerEvent { t, d });
}

impl<T: NodeTransport + 'static> domain::emit::UserEmitter for UserRouter<T> {
    fn emit_to_users(
        &self,
        users: &[domain::id::UserId],
        t: String,
        payload: String,
    ) -> domain::emit::BoxFuture<'_, ()> {
        let ids: Vec<u64> = users.iter().map(|u| u.0.raw()).collect();
        Box::pin(async move {
            // 로컬 배달 (이 노드 세션 — detach 버퍼 포함). 로컬 세션 없는 유저는 Hub가 자동 스킵.
            let d: Value = serde_json::from_str(&payload).unwrap_or(Value::Null);
            self.hub.deliver(&ids, &ServerEvent { t: t.clone(), d });

            // 원격 배달: 대상 유저를 호스팅하는 (로컬 제외) 노드별로 묶어 USER_DELIVER 타깃 전송.
            let mut by_node: HashMap<u64, Vec<u64>> = HashMap::new();
            for &u in &ids {
                for node in self.presence.nodes_for(u) {
                    if node != self.local_node_id {
                        by_node.entry(node).or_default().push(u);
                    }
                }
            }
            for (node, user_ids) in by_node {
                self.router
                    .send_to(
                        node,
                        NodeMessage::UserDeliver { t: t.clone(), payload: payload.clone(), user_ids },
                    )
                    .await;
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::emit::UserEmitter;
    use domain::id::{Snowflake, SnowflakeGenerator, UserId};
    use node::clock::{Clock, SystemClock};
    use node::{HashRing, RealmEvent, Status};
    use tokio::sync::mpsc;
    use transport::Switchboard;

    fn uid(n: u64) -> UserId {
        UserId(Snowflake::from_raw(n))
    }

    /// D43 종단(유닛): 한 emit이 로컬 유저는 Hub로, 원격 유저는 호스팅 노드에 USER_DELIVER로 배달.
    #[tokio::test]
    async fn emit_routes_local_and_remote() {
        let board = Switchboard::new();
        let (t1, _r1) = board.join(1, 16); // 로컬 노드 1
        let (_t2, mut r2) = board.join(2, 16); // 원격 노드 2 (수신 검증용)

        let mut ring = HashRing::new(64);
        ring.add_node(1);
        ring.add_node(2);
        let (etx, _erx) = mpsc::channel::<RealmEvent>(16);
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let router = Arc::new(Router::new(1, Arc::new(SnowflakeGenerator::new(1)), clock, ring, t1, etx));

        let hub = Hub::new();
        let presence = Arc::new(Presence::new());

        // 로컬 유저 0xA: 이 노드(1)에 세션 보유 + presence 호스팅=노드1.
        let (mut rx_a, _tok) = hub.attach(0xA, 100, 16);
        hub.activate(0xA, 100);
        presence.set(0xA, 1, Status::Online);
        // 원격 유저 0xB: 노드2에 호스팅(이 노드엔 세션 없음).
        presence.set(0xB, 2, Status::Online);

        let ur = UserRouter::new(hub.clone(), Arc::clone(&presence), Arc::clone(&router), 1);
        let payload = r#"{"user":{"id":"99"},"kind":"pending_in"}"#.to_string();
        ur.emit_to_users(&[uid(0xA), uid(0xB)], "RELATIONSHIP_ADD".into(), payload.clone()).await;

        // 로컬 유저 0xA는 Hub 세션으로 직접 수신.
        let frame = rx_a.recv().await.unwrap();
        assert_eq!(frame.t.as_deref(), Some("RELATIONSHIP_ADD"));

        // 원격 유저 0xB는 노드2에 USER_DELIVER로 타깃 전송.
        let ib = r2.recv().await.unwrap();
        assert_eq!(ib.src, 1);
        match ib.msg {
            NodeMessage::UserDeliver { t, payload: p, user_ids } => {
                assert_eq!(t, "RELATIONSHIP_ADD");
                assert_eq!(p, payload);
                assert_eq!(user_ids, vec![0xB]); // 0xA는 로컬이라 원격 전송 대상 아님
            }
            other => panic!("expected UserDeliver, got {other:?}"),
        }
    }

    /// 오프라인(디렉터리에 없는) 유저는 원격 전송 0 — Hub도 로컬 세션 없어 no-op(무해).
    #[tokio::test]
    async fn offline_user_no_remote_send() {
        let board = Switchboard::new();
        let (t1, _r1) = board.join(1, 16);
        let (_t2, mut r2) = board.join(2, 16);
        let mut ring = HashRing::new(64);
        ring.add_node(1);
        ring.add_node(2);
        let (etx, _erx) = mpsc::channel::<RealmEvent>(16);
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let router = Arc::new(Router::new(1, Arc::new(SnowflakeGenerator::new(1)), clock, ring, t1, etx));

        let ur = UserRouter::new(Hub::new(), Arc::new(Presence::new()), router, 1);
        ur.emit_to_users(&[uid(0xDEAD)], "MESSAGE_ACK".into(), "{}".into()).await;

        // 노드2엔 아무것도 안 옴.
        assert!(r2.try_recv().is_err(), "오프라인 유저는 원격 전송 없음");
    }
}
