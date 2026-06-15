//! Gateway 라우트 (개념: routes). WS 업그레이드 + 메시지 전송(REST).
//!
//! Discord 모델: 메시지 **전송은 REST**(`POST /channels/:id/messages`)로 받아 Router로 보내고,
//! 결과 `MESSAGE_CREATE`는 **gateway(WS)로 push**. 전송자는 자기 세션으로 에코받음(D13 구독).

use axum::Json;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::{get, post};
use domain::id::{ChannelId, MessageId, Snowflake};
use domain::repo::Store;
use serde::{Deserialize, Serialize};
use transport::NodeTransport;

use crate::session::handle_socket;
use crate::state::GatewayState;

pub fn router<S: Store + 'static, T: NodeTransport>(
    state: GatewayState<S, T>,
) -> axum::Router {
    axum::Router::new()
        .route("/gateway", get(ws_upgrade::<S, T>))
        .route("/channels/{channel_id}/messages", post(send_message::<S, T>))
        .with_state(state)
}

async fn ws_upgrade<S: Store + 'static, T: NodeTransport>(
    State(state): State<GatewayState<S, T>>,
    ws: WebSocketUpgrade,
) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

#[derive(Deserialize)]
struct SendMessageReq {
    content: String,
    nonce: Option<String>,
    /// 답장 대상 메시지 id (문자열, D39). 같은 채널의 살아있는 메시지여야.
    reference_message_id: Option<String>,
}

#[derive(Serialize)]
struct SendAck {
    /// 접수됨 — 결과 메시지는 gateway `MESSAGE_CREATE`로 도착.
    queued: bool,
    nonce: Option<String>,
}

/// 인증된 유저가 채널에 메시지 전송. 멤버십 검사 후 Router로 전달(fire-and-forget).
async fn send_message<S: Store + 'static, T: NodeTransport>(
    State(state): State<GatewayState<S, T>>,
    AuthBearer(user): AuthBearer,
    Path(channel_id): Path<String>,
    Json(req): Json<SendMessageReq>,
) -> Result<(StatusCode, Json<SendAck>), (StatusCode, &'static str)> {
    let channel_id = channel_id
        .parse::<u64>()
        .map(|n| ChannelId(Snowflake::from_raw(n)))
        .map_err(|_| (StatusCode::BAD_REQUEST, "invalid channel id"))?;
    if req.content.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "empty content"));
    }

    // 채널 → realm 해석 + SEND_MESSAGES 권한 검사 (D17, 서버가 신뢰 경계).
    let channel = state
        .store
        .get(channel_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "db error"))?
        .ok_or((StatusCode::NOT_FOUND, "channel not found"))?;
    if !can_send(&*state.store, channel_id, channel.realm_id, user).await? {
        return Err((StatusCode::FORBIDDEN, "missing SEND_MESSAGES"));
    }

    // 1:1 DM 차단 게이팅 (permissions.md §5): 어느 한쪽이라도 차단했으면 전송 거부.
    if channel.kind == domain::channel::ChannelKind::Dm {
        let db = |_| (StatusCode::INTERNAL_SERVER_ERROR, "db error");
        if let Some(info) = state.store.get_realm(channel.realm_id).await.map_err(db)?
            && info.kind == domain::dm::RealmKind::Dm
        {
            for m in state.store.list_members(channel.realm_id).await.map_err(db)? {
                if m.user_id != user && state.store.is_blocked_between(user, m.user_id).await.map_err(db)? {
                    return Err((StatusCode::FORBIDDEN, "blocked"));
                }
            }
        }
    }

    // 답장 대상 검증 (D39): 같은 채널의 살아있는 메시지여야.
    let reference_message_id = match &req.reference_message_id {
        Some(s) => {
            let rid = s
                .parse::<u64>()
                .map(|n| MessageId(Snowflake::from_raw(n)))
                .map_err(|_| (StatusCode::BAD_REQUEST, "invalid reference_message_id"))?;
            let refd = state
                .store
                .get_message(rid)
                .await
                .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "db error"))?
                .ok_or((StatusCode::BAD_REQUEST, "reference message not found"))?;
            if refd.channel_id != channel_id {
                return Err((StatusCode::BAD_REQUEST, "reference message in different channel"));
            }
            Some(rid)
        }
        None => None,
    };

    // Router로 전달 → Realm 액터가 ID·순서 확정 → dispatch 드라이버가 persist+fanout.
    state
        .router
        .route_send(channel.realm_id, channel_id, user, req.content, req.nonce.clone(), reference_message_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "route failed"))?;

    Ok((StatusCode::ACCEPTED, Json(SendAck { queued: true, nonce: req.nonce })))
}

/// 멤버이면서 **채널 컨텍스트**(오버라이드 적용)에서 SEND_MESSAGES 권한이 있는가 (D17). 계산은 domain.
async fn can_send<S: Store>(
    store: &S,
    channel_id: domain::id::ChannelId,
    realm: domain::id::RealmId,
    user: UserId,
) -> Result<bool, (StatusCode, &'static str)> {
    use domain::permissions::{Permissions, effective_channel_permissions};
    let db = |_| (StatusCode::INTERNAL_SERVER_ERROR, "db error");
    if !store.is_member(realm, user).await.map_err(db)? {
        return Ok(false);
    }
    let is_owner = store.get_guild(realm).await.map_err(db)?.map(|g| g.owner_id == user).unwrap_or(false);
    let everyone = store
        .everyone_permissions(realm)
        .await
        .map_err(db)?
        .map(Permissions::from_bits_truncate)
        .unwrap_or_else(Permissions::default_everyone);
    let member_roles: Vec<(u64, Permissions)> = store
        .member_roles_with_ids(realm, user)
        .await
        .map_err(db)?
        .into_iter()
        .map(|(id, bits)| (id, Permissions::from_bits_truncate(bits)))
        .collect();
    let overwrites = store.list_overwrites(channel_id).await.map_err(db)?;
    let perms = effective_channel_permissions(
        is_owner,
        realm.0.raw(),
        user.0.raw(),
        everyone,
        &member_roles,
        &overwrites,
    );
    Ok(perms.contains(Permissions::SEND_MESSAGES))
}

// --- 인증 추출기 (gateway 로컬: rest-api와 독립) ---

use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use domain::id::UserId;

struct AuthBearer(UserId);

impl<S: Store + 'static, T: NodeTransport> FromRequestParts<GatewayState<S, T>> for AuthBearer {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &GatewayState<S, T>,
    ) -> Result<Self, Self::Rejection> {
        let token = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.strip_prefix("Bearer "))
            .ok_or((StatusCode::UNAUTHORIZED, "missing bearer token"))?;
        let uid = state
            .keys
            .verify_access(token)
            .map_err(|_| (StatusCode::UNAUTHORIZED, "invalid token"))?;
        Ok(AuthBearer(UserId(Snowflake::from_raw(uid))))
    }
}
