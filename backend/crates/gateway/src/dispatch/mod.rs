//! 디스패치 드라이버 (개념: dispatch). Realm 액터 이벤트 → persist → 팬아웃 → 세션 배달.
//!
//! D24 persist-then-fanout: Realm 액터가 단일 소유자로 ID·순서를 확정해 이벤트를 방출하면,
//! **단일 소비자**인 이 루프가 (1) Postgres 저장 → (2) Router::fanout(노드별 대상 산출) →
//! (3) Hub로 로컬 세션에 push 한다. node 코어를 IO 무의존으로 유지(P2)하면서 순서는 보존
//! (단일 액터가 순서대로 방출 → 단일 소비자가 순서대로 persist).

use std::sync::Arc;

use domain::message::NewMessage;
use domain::repo::Store;
use node::{LocalDelivery, RealmEvent, Router};
use tokio::sync::mpsc;
use transport::NodeTransport;

use crate::hub::Hub;
use crate::protocol::ServerEvent;

/// LocalDelivery → MESSAGE_CREATE JSON 페이로드(`d`). id는 문자열(Discord 관례).
fn message_create_payload(d: &LocalDelivery) -> serde_json::Value {
    serde_json::json!({
        "id": d.message_id.0.raw().to_string(),
        "channel_id": d.channel_id.0.raw().to_string(),
        "author": { "id": d.author.0.raw().to_string() },
        "content": d.content,
        "nonce": d.nonce,
    })
}

/// LocalDelivery를 로컬 세션들에 MESSAGE_CREATE로 배달.
/// dispatch 드라이버(로컬 소유)와 크로스노드 inbound 루프(원격 RealmFanout 수신) 양쪽이 사용.
pub async fn deliver_local(hub: &Hub, d: &LocalDelivery) {
    let event = ServerEvent { t: "MESSAGE_CREATE".into(), d: message_create_payload(d) };
    hub.deliver(&d.user_ids, &event).await;
}

/// 이벤트 루프. server가 `tokio::spawn`으로 구동. 모든 송신측 drop 시 종료.
pub async fn run_dispatch<S: Store + 'static, T: NodeTransport>(
    mut events: mpsc::Receiver<RealmEvent>,
    router: Arc<Router<T>>,
    store: Arc<S>,
    hub: Hub,
) {
    while let Some(ev) = events.recv().await {
        let RealmEvent::MessageCreated {
            realm,
            channel_id,
            message_id,
            author,
            content,
            nonce,
            ..
        } = &ev;

        // (1) persist (D24). nonce 중복이면 멱등 스킵 (D34).
        let new = NewMessage {
            id: *message_id,
            channel_id: *channel_id,
            realm_id: *realm,
            author_id: *author,
            content: content.clone(),
            nonce: nonce.clone(),
        };
        match store.create_message(&new).await {
            Ok(true) => {}
            Ok(false) => continue, // 같은 nonce 재전송 — 이미 배달됨.
            Err(e) => {
                tracing::error!(error = %e, "message persist failed; skipping fanout");
                continue;
            }
        }

        // (2) 팬아웃 대상 산출 (D12).
        let deliveries = match router.fanout(ev).await {
            Ok(d) => d,
            Err(e) => {
                tracing::error!(error = %e, "fanout failed");
                continue;
            }
        };

        // (3) 로컬 세션 배달.
        for d in deliveries {
            deliver_local(&hub, &d).await;
        }
    }
}
