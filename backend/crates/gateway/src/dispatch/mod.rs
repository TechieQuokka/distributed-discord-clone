//! 디스패치 드라이버 (개념: dispatch). Realm 액터 이벤트 → (메시지면 persist) → 팬아웃 → 세션 배달.
//!
//! D24 persist-then-fanout + D39 범용 envelope: Realm 액터가 단일 소유자로 ID·순서를 확정해
//! 이벤트를 방출하면, **단일 소비자**인 이 루프가 처리한다.
//! - `MessageCreated`: (1) Postgres 저장 → (2) MESSAGE_CREATE JSON 조립 → (3) `Router::fanout`.
//! - `Broadcast`(멤버 변동 등): persist 없이 payload(이미 직렬화된 JSON)를 그대로 `Router::fanout`.
//!
//! JSON 단일 출처 = 생산 엣지(여기/REST). node·protocol은 payload를 불투명 문자열로 통과(P2).

use std::sync::Arc;

use domain::message::NewMessage;
use domain::repo::Store;
use node::{LocalDelivery, RealmEvent, Router};
use tokio::sync::mpsc;
use transport::NodeTransport;

use crate::hub::Hub;
use crate::protocol::ServerEvent;

/// MESSAGE_CREATE JSON 페이로드(`d`). id는 문자열(Discord 관례). reference/mentions 포함(D39).
#[allow(clippy::too_many_arguments)]
fn message_create_payload(
    message_id: u64,
    channel_id: u64,
    author: u64,
    content: &str,
    nonce: &Option<String>,
    reference_message_id: Option<u64>,
    mentions: &[u64],
) -> serde_json::Value {
    serde_json::json!({
        "id": message_id.to_string(),
        "channel_id": channel_id.to_string(),
        "author": { "id": author.to_string() },
        "content": content,
        "nonce": nonce,
        "reference_message_id": reference_message_id.map(|r| r.to_string()),
        "mentions": mentions.iter().map(|m| m.to_string()).collect::<Vec<_>>(),
    })
}

/// LocalDelivery를 로컬 세션들에 배달 (범용 envelope, D39). payload(JSON 문자열)를 1회 역파싱.
/// dispatch 드라이버(로컬 소유)와 크로스노드 inbound 루프(원격 RealmFanout 수신) 양쪽이 사용.
pub async fn deliver_local(hub: &Hub, d: &LocalDelivery) {
    let payload: serde_json::Value =
        serde_json::from_str(&d.payload).unwrap_or(serde_json::Value::Null);
    let event = ServerEvent { t: d.t.clone(), d: payload };
    hub.deliver(&d.user_ids, &event);
}

/// 이벤트 루프. server가 `tokio::spawn`으로 구동. 모든 송신측 drop 시 종료.
pub async fn run_dispatch<S: Store + 'static, T: NodeTransport>(
    mut events: mpsc::Receiver<RealmEvent>,
    router: Arc<Router<T>>,
    store: Arc<S>,
    hub: Hub,
) {
    while let Some(ev) = events.recv().await {
        let (realm, t, payload, targets) = match ev {
            RealmEvent::MessageCreated {
                realm,
                channel_id,
                message_id,
                author,
                content,
                nonce,
                reference_message_id,
                targets,
            } => {
                // (1) persist (D24). nonce 중복이면 멱등 스킵 (D34).
                let new = NewMessage {
                    id: message_id,
                    channel_id,
                    realm_id: realm,
                    author_id: author,
                    content: content.clone(),
                    nonce: nonce.clone(),
                    reference_message_id,
                };
                match store.create_message(&new).await {
                    Ok(true) => {}
                    Ok(false) => continue, // 같은 nonce 재전송 — 이미 배달됨.
                    Err(e) => {
                        tracing::error!(error = %e, "message persist failed; skipping fanout");
                        continue;
                    }
                }
                // 이벤트 소싱(D48): 메시지 생성 사실을 append-only 로그에 기록. 단일 직렬 소비자(D24)라
                // per-realm seq 경합 없음. 로그는 messages(엔티티 진실)와 별개의 사실 스트림(CQRS).
                // append 실패는 warn하고 계속(배달은 막지 않음 — messages가 진실, 로그는 보조). seam: 완전
                // 무결성은 persist와 한 트랜잭션(후속).
                if let Err(e) = store
                    .append_event(
                        realm,
                        &domain::event::RealmEventKind::MessageCreated {
                            message_id,
                            channel_id,
                            author,
                        },
                    )
                    .await
                {
                    tracing::warn!(error = %e, "event log append failed (continuing)");
                }
                // (2) 멘션 파싱(content 파생, D39) → 적재. 존재 유저만(어댑터 보장).
                let mentions = domain::mention::parse_mentions(&content);
                if let Err(e) = store.add_mentions(message_id, &mentions).await {
                    tracing::warn!(error = %e, "mention persist failed (continuing)");
                }
                // 안 읽은 멘션 카운트 +1 (작성자 본인 제외). 새 메시지는 항상 최신 → 단순 증가가 정확.
                let bump: Vec<domain::id::UserId> =
                    mentions.iter().copied().filter(|u| *u != author).collect();
                if let Err(e) = store.bump_mentions(channel_id, &bump).await {
                    tracing::warn!(error = %e, "mention count bump failed (continuing)");
                }
                let mention_ids: Vec<u64> = mentions.iter().map(|u| u.0.raw()).collect();

                // (3) MESSAGE_CREATE JSON 조립 (생산 엣지가 단일 출처, D39).
                let payload = message_create_payload(
                    message_id.0.raw(),
                    channel_id.0.raw(),
                    author.0.raw(),
                    &content,
                    &nonce,
                    reference_message_id.map(|r| r.0.raw()),
                    &mention_ids,
                )
                .to_string();
                (realm, "MESSAGE_CREATE".to_string(), payload, targets)
            }
            // 비-메시지 이벤트: persist 없이 그대로 (payload는 REST 엣지가 이미 직렬화, D39).
            RealmEvent::Broadcast { realm, t, payload, targets } => (realm, t, payload, targets),
        };

        // 팬아웃 대상 산출 (D12) + 로컬 세션 배달.
        let deliveries = match router.fanout(realm, t, payload, targets).await {
            Ok(d) => d,
            Err(e) => {
                tracing::error!(error = %e, "fanout failed");
                continue;
            }
        };
        for d in deliveries {
            deliver_local(&hub, &d).await;
        }
    }
}
