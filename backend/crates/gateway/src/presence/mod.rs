//! 전역 presence 오케스트레이션 (개념: presence). Q11/D12 — D40/D41의 크로스노드 seam을 닫는 경로.
//!
//! 흐름: 로컬 세션 온/오프라인 전이 → presence 레지스트리 갱신 + 피어에 `PRESENCE_GOSSIP` 브로드캐스트
//!       + 그 유저의 **로컬 친구**에게 `PRESENCE_UPDATE` 배달. 피어가 gossip을 받으면 자기 view 갱신 후
//!       마찬가지로 로컬 친구에게 배달(풀메시 broadcast + 로컬 필터 = 크로스노드 유저 라우팅).
//!
//! Realm 팬아웃(D12 구독자표)과 분리된 전역 유저 경로 — 친구 그래프(relationships)로 대상 산출.

use domain::id::{Snowflake, UserId};
use domain::relationship::RelationKind;
use domain::repo::Store;
use node::{Presence, Router, Status};
use protocol::NodeMessage;
use serde_json::{Value, json};
use transport::NodeTransport;

use crate::hub::Hub;
use crate::protocol::ServerEvent;

/// 로컬 유저가 `status`로 전이(online/idle/dnd): 레지스트리 set + gossip 브로드캐스트 + 로컬 친구 통지.
/// 클라 op 3(PRESENCE_UPDATE)와 세션 온라인 전이가 공유하는 경로 (D42).
pub async fn set_status<S: Store, T: NodeTransport>(
    presence: &Presence,
    hub: &Hub,
    store: &S,
    router: &Router<T>,
    local_node: u64,
    user: u64,
    status: Status,
) {
    let changed = presence.set(user, local_node, status);
    router
        .broadcast(NodeMessage::PresenceGossip {
            user_id: user,
            node_id: local_node,
            status: status.as_u8(),
        })
        .await;
    if changed {
        notify_friends(hub, store, presence, user).await;
    }
}

/// 로컬 유저 온라인 전이(세션 첫 live): `set_status(Online)`의 단축.
pub async fn set_online<S: Store, T: NodeTransport>(
    presence: &Presence,
    hub: &Hub,
    store: &S,
    router: &Router<T>,
    local_node: u64,
    user: u64,
) {
    set_status(presence, hub, store, router, local_node, user, Status::Online).await;
}

/// 로컬 유저 오프라인 전이 (마지막 live 세션 종료): 레지스트리 clear + gossip + 로컬 친구 통지.
pub async fn set_offline<S: Store, T: NodeTransport>(
    presence: &Presence,
    hub: &Hub,
    store: &S,
    router: &Router<T>,
    local_node: u64,
    user: u64,
) {
    let changed = presence.clear(user, local_node);
    router
        .broadcast(NodeMessage::PresenceGossip {
            user_id: user,
            node_id: local_node,
            status: Status::Offline.as_u8(),
        })
        .await;
    if changed {
        notify_friends(hub, store, presence, user).await;
    }
}

/// 피어로부터 받은 gossip 적용 (재브로드캐스트 없음 — 풀메시라 원본 노드가 이미 전 피어에 전송).
/// 유효 상태가 바뀌면 그 유저의 로컬 친구에게 통지.
pub async fn apply_gossip<S: Store>(
    presence: &Presence,
    hub: &Hub,
    store: &S,
    user: u64,
    node: u64,
    status: u8,
) {
    let changed = if status == Status::Offline.as_u8() {
        presence.clear(user, node)
    } else {
        presence.set(user, node, Status::from_u8(status))
    };
    if changed {
        notify_friends(hub, store, presence, user).await;
    }
}

/// `user`의 친구들 중 **이 노드에 세션이 있는** 이들에게 `PRESENCE_UPDATE` 배달.
/// `hub.deliver`가 로컬 세션 없는 유저는 자동 스킵 → 친구 id 전체를 넘겨도 안전.
async fn notify_friends<S: Store>(hub: &Hub, store: &S, presence: &Presence, user: u64) {
    let Ok(rels) = store.list_relationships(uid(user)).await else { return };
    let friend_ids: Vec<u64> = rels
        .iter()
        .filter(|r| r.kind == RelationKind::Friend)
        .map(|r| r.target_id.0.raw())
        .collect();
    if friend_ids.is_empty() {
        return;
    }
    let status = presence.get(user);
    let event = ServerEvent {
        t: "PRESENCE_UPDATE".into(),
        d: json!({ "user": { "id": user.to_string() }, "status": status.as_str() }),
    };
    hub.deliver(&friend_ids, &event);
}

/// READY 스냅샷용: 내 친구들 중 **현재 온라인**인 이들의 presence 목록.
pub async fn ready_presences<S: Store>(store: &S, presence: &Presence, me: u64) -> Vec<Value> {
    let Ok(rels) = store.list_relationships(uid(me)).await else { return Vec::new() };
    rels.iter()
        .filter(|r| r.kind == RelationKind::Friend)
        .filter_map(|r| {
            let f = r.target_id.0.raw();
            let s = presence.get(f);
            (s != Status::Offline)
                .then(|| json!({ "user": { "id": f.to_string() }, "status": s.as_str() }))
        })
        .collect()
}

fn uid(raw: u64) -> UserId {
    UserId(Snowflake::from_raw(raw))
}
