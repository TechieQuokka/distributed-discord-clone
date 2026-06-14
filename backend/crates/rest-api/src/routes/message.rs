//! 메시지 히스토리 라우트 (개념: routes/message). `GET /channels/:id/messages` (D38).
//!
//! 메시지 **전송**은 실시간 경로(Gateway WS)로 처리. 여기선 히스토리 조회만.

use axum::Json;
use axum::extract::{Path, Query, State};
use domain::id::{ChannelId, MessageId, Snowflake};
use domain::repo::Store;
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::extract::AuthUser;
use crate::state::AppState;

pub fn routes<S: Store + 'static>() -> axum::Router<AppState<S>> {
    axum::Router::new().route(
        "/channels/{channel_id}/messages",
        axum::routing::get(list_messages::<S>),
    )
}

#[derive(Deserialize)]
pub struct HistoryQuery {
    /// Snowflake 커서: 이 id 이전(더 과거) 메시지 (D38).
    pub before: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Serialize)]
pub struct MessageView {
    pub id: String,
    pub channel_id: String,
    pub author_id: String,
    pub content: String,
}

async fn list_messages<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(user): AuthUser,
    Path(channel_id): Path<String>,
    Query(q): Query<HistoryQuery>,
) -> Result<Json<Vec<MessageView>>, ApiError> {
    let channel_id = channel_id
        .parse::<u64>()
        .map(|n| ChannelId(Snowflake::from_raw(n)))
        .map_err(|_| ApiError::BadRequest("invalid channel id".into()))?;

    // 채널 → realm 확인 후 채널 권한 검사 (D17): 히스토리 조회는 VIEW_CHANNEL + READ_MESSAGE_HISTORY 필요.
    let channel = st.store.get(channel_id).await?.ok_or(ApiError::NotFound)?;
    crate::perm::require_in_channel(
        &*st.store,
        channel_id,
        channel.realm_id,
        user,
        domain::permissions::Permissions::VIEW_CHANNEL | domain::permissions::Permissions::READ_MESSAGE_HISTORY,
    )
    .await?;

    let before = match q.before {
        Some(s) => Some(
            s.parse::<u64>()
                .map(|n| MessageId(Snowflake::from_raw(n)))
                .map_err(|_| ApiError::BadRequest("invalid before cursor".into()))?,
        ),
        None => None,
    };
    let limit = q.limit.unwrap_or(50);

    let msgs = st.store.list_by_channel(channel_id, before, limit).await?;
    Ok(Json(
        msgs.into_iter()
            .map(|m| MessageView {
                id: m.id.0.raw().to_string(),
                channel_id: m.channel_id.0.raw().to_string(),
                author_id: m.author_id.0.raw().to_string(),
                content: m.content,
            })
            .collect(),
    ))
}
