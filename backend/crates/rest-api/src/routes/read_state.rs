//! 읽음 상태 라우트 (개념: read_state). 02-schema §8, gateway `MESSAGE_ACK`.
//!
//! - `POST /channels/{cid}/messages/{mid}/ack` — 채널을 그 메시지까지 읽음 처리(+멘션수 재계산).
//! - `GET /users/@me/read-states` — 내 읽음 상태 목록(READY 스냅샷과 동일 내용, CLI/테스트 편의).
//!
//! ack는 **유저 단위** `MESSAGE_ACK`를 본인 세션들에 통지(다른 기기 동기화) — `UserEmitter`(D40 재사용).

use axum::Json;
use axum::extract::{Path, State};
use axum::routing::{get, post};
use domain::id::{ChannelId, MessageId, Snowflake};
use domain::permissions::Permissions;
use domain::read_state::ReadState;
use domain::repo::Store;
use serde::Serialize;

use crate::error::ApiError;
use crate::events::message_ack_payload;
use crate::extract::AuthUser;
use crate::state::AppState;

pub fn routes<S: Store + 'static>() -> axum::Router<AppState<S>> {
    axum::Router::new()
        .route("/channels/{channel_id}/messages/{message_id}/ack", post(ack::<S>))
        .route("/users/@me/read-states", get(list_read_states::<S>))
}

#[derive(Serialize)]
pub struct ReadStateView {
    pub channel_id: String,
    pub last_read_message_id: Option<String>,
    pub mention_count: i32,
}

impl From<ReadState> for ReadStateView {
    fn from(s: ReadState) -> Self {
        ReadStateView {
            channel_id: s.channel_id.0.raw().to_string(),
            last_read_message_id: s.last_read_message_id.map(|m| m.0.raw().to_string()),
            mention_count: s.mention_count,
        }
    }
}

/// 채널을 message까지 읽음 처리 → `MESSAGE_ACK` 통지.
async fn ack<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(user): AuthUser,
    Path((channel_id, message_id)): Path<(String, String)>,
) -> Result<Json<ReadStateView>, ApiError> {
    let cid = channel_id
        .parse::<u64>()
        .map(|n| ChannelId(Snowflake::from_raw(n)))
        .map_err(|_| ApiError::BadRequest("invalid channel id".into()))?;
    let mid = message_id
        .parse::<u64>()
        .map(|n| MessageId(Snowflake::from_raw(n)))
        .map_err(|_| ApiError::BadRequest("invalid message id".into()))?;

    // 채널 조회 권한이 있어야 읽음 처리 가능 (채널 컨텍스트 VIEW_CHANNEL, DM은 default_everyone 폴백).
    let channel = st.store.get(cid).await?.ok_or(ApiError::NotFound)?;
    crate::perm::require_in_channel(&*st.store, cid, channel.realm_id, user, Permissions::VIEW_CHANNEL).await?;

    // 대상 메시지는 그 채널의 살아있는 메시지여야.
    let msg = st.store.get_message(mid).await?.ok_or(ApiError::NotFound)?;
    if msg.channel_id != cid {
        return Err(ApiError::NotFound);
    }

    let state = st.store.ack(user, cid, mid).await?;
    let payload = message_ack_payload(cid, mid, state.mention_count);
    let _ = st.user_emitter.emit_to_users(&[user], "MESSAGE_ACK".into(), payload).await;
    Ok(Json(state.into()))
}

async fn list_read_states<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(user): AuthUser,
) -> Result<Json<Vec<ReadStateView>>, ApiError> {
    let states = st.store.list_read_states(user).await?;
    Ok(Json(states.into_iter().map(ReadStateView::from).collect()))
}
