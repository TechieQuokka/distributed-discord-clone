//! 길드/채널 라우트 (개념: routes/guild). `/guilds`, `/guilds/:id/channels`.
//!
//! 길드 생성 시 소유자가 자동 멤버 + 기본 'general' 텍스트 채널 1개 생성.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::post;
use domain::channel::{ChannelKind, NewChannel};
use domain::guild::NewGuild;
use domain::id::{ChannelId, RealmId, Snowflake};
use domain::repo::Store;
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::extract::AuthUser;
use crate::state::AppState;

pub fn routes<S: Store + 'static>() -> axum::Router<AppState<S>> {
    axum::Router::new()
        .route("/guilds", post(create_guild::<S>))
        .route("/guilds/{realm_id}/channels", post(create_channel::<S>))
}

#[derive(Deserialize)]
pub struct CreateGuildReq {
    pub name: String,
}

#[derive(Deserialize)]
pub struct CreateChannelReq {
    pub name: String,
}

#[derive(Serialize)]
pub struct ChannelView {
    pub id: String,
    pub name: Option<String>,
    pub kind: String,
}

#[derive(Serialize)]
pub struct GuildView {
    pub id: String,
    pub name: String,
    pub channels: Vec<ChannelView>,
}

/// 길드 id 문자열(Snowflake) 파싱.
fn parse_realm(s: &str) -> Result<RealmId, ApiError> {
    s.parse::<u64>()
        .map(|n| RealmId(Snowflake::from_raw(n)))
        .map_err(|_| ApiError::BadRequest("invalid guild id".into()))
}

async fn create_guild<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(owner): AuthUser,
    Json(req): Json<CreateGuildReq>,
) -> Result<(StatusCode, Json<GuildView>), ApiError> {
    let name = req.name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest("guild name is required".into()));
    }

    let realm_id = RealmId(st.snowflakes.next(st.clock.now_ms()));
    st.store
        .create_guild(&NewGuild { realm_id, name: name.to_owned(), owner_id: owner })
        .await?;

    // 기본 채널.
    let chan_id = ChannelId(st.snowflakes.next(st.clock.now_ms()));
    st.store
        .create_channel(&NewChannel {
            id: chan_id,
            realm_id,
            kind: ChannelKind::Text,
            name: "general".into(),
        })
        .await?;

    Ok((
        StatusCode::CREATED,
        Json(GuildView {
            id: realm_id.0.raw().to_string(),
            name: name.to_owned(),
            channels: vec![ChannelView {
                id: chan_id.0.raw().to_string(),
                name: Some("general".into()),
                kind: "text".into(),
            }],
        }),
    ))
}

async fn create_channel<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(user): AuthUser,
    Path(realm_id): Path<String>,
    Json(req): Json<CreateChannelReq>,
) -> Result<(StatusCode, Json<ChannelView>), ApiError> {
    let realm_id = parse_realm(&realm_id)?;
    let name = req.name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest("channel name is required".into()));
    }
    // 멤버만 채널 생성 가능.
    if !st.store.is_member(realm_id, user).await? {
        return Err(ApiError::Forbidden);
    }

    let id = ChannelId(st.snowflakes.next(st.clock.now_ms()));
    st.store
        .create_channel(&NewChannel { id, realm_id, kind: ChannelKind::Text, name: name.to_owned() })
        .await?;

    Ok((
        StatusCode::CREATED,
        Json(ChannelView { id: id.0.raw().to_string(), name: Some(name.to_owned()), kind: "text".into() }),
    ))
}
