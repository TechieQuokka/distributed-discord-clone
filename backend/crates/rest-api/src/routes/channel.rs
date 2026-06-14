//! 채널 권한 오버라이드 라우트 (개념: routes/channel). D17 채널별 allow/deny.
//!
//! `PUT /channels/:id/permissions/:target_id` — 역할/멤버 오버라이드 설정. MANAGE_ROLES 필요.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::put;
use domain::id::{ChannelId, Snowflake};
use domain::permissions::{ChannelOverwrite, OverwriteKind, Permissions};
use domain::repo::Store;
use serde::Deserialize;

use crate::error::ApiError;
use crate::extract::AuthUser;
use crate::perm;
use crate::state::AppState;

pub fn routes<S: Store + 'static>() -> axum::Router<AppState<S>> {
    axum::Router::new()
        .route("/channels/{channel_id}/permissions/{target_id}", put(set_overwrite::<S>))
}

#[derive(Deserialize)]
pub struct SetOverwriteReq {
    /// "role" | "member".
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub allow: u64,
    #[serde(default)]
    pub deny: u64,
}

async fn set_overwrite<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(actor): AuthUser,
    Path((channel_id, target_id)): Path<(String, String)>,
    Json(req): Json<SetOverwriteReq>,
) -> Result<StatusCode, ApiError> {
    let channel_id = ChannelId(Snowflake::from_raw(
        channel_id.parse::<u64>().map_err(|_| ApiError::BadRequest("invalid channel id".into()))?,
    ));
    let target_id =
        target_id.parse::<u64>().map_err(|_| ApiError::BadRequest("invalid target id".into()))?;
    let kind = OverwriteKind::parse(&req.kind)
        .ok_or_else(|| ApiError::BadRequest("type must be 'role' or 'member'".into()))?;

    let channel = st.store.get(channel_id).await?.ok_or(ApiError::NotFound)?;
    // 채널 권한 편집은 MANAGE_ROLES 필요 (Discord 관례).
    perm::require(&*st.store, channel.realm_id, actor, Permissions::MANAGE_ROLES).await?;

    st.store
        .set_overwrite(&ChannelOverwrite {
            channel_id,
            target_id,
            kind,
            allow: Permissions::from_bits_truncate(req.allow),
            deny: Permissions::from_bits_truncate(req.deny),
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
