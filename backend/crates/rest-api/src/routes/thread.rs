//! 스레드 라우트 (개념: routes/thread). Phase 4.
//!
//! 스레드 = 부모 채널과 같은 Realm의 channels(kind='thread') 한 행(P4) — 메시징/팬아웃은 길드와 동일.
//! 여기선 생성(부모 채널 컨텍스트 CREATE_PUBLIC_THREADS)·목록(VIEW_CHANNEL)·아카이브(소유자/MANAGE_THREADS).
//! 생성/변경은 `THREAD_CREATE`/`THREAD_UPDATE`로 Realm 구독자에 팬아웃(D39).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, patch};
use domain::id::{ChannelId, Snowflake};
use domain::permissions::Permissions;
use domain::repo::Store;
use domain::thread::{NewThread, Thread};
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::events::thread_payload;
use crate::extract::AuthUser;
use crate::state::AppState;

pub fn routes<S: Store + 'static>() -> axum::Router<AppState<S>> {
    axum::Router::new()
        .route(
            "/channels/{channel_id}/threads",
            get(list_threads::<S>).post(create_thread::<S>),
        )
        .route("/channels/{channel_id}/thread", patch(update_thread::<S>))
}

fn parse_channel(s: &str) -> Result<ChannelId, ApiError> {
    s.parse::<u64>()
        .map(|n| ChannelId(Snowflake::from_raw(n)))
        .map_err(|_| ApiError::BadRequest("invalid channel id".into()))
}

#[derive(Deserialize)]
pub struct CreateThreadReq {
    pub name: String,
    /// 자동 아카이브(분). 기본 1440.
    pub auto_archive: Option<i32>,
}

#[derive(Serialize)]
pub struct ThreadView {
    pub id: String,
    pub realm_id: String,
    pub parent_id: String,
    pub name: Option<String>,
    pub owner_id: Option<String>,
    pub archived: bool,
    pub auto_archive: i32,
    pub message_count: i64,
}

impl From<Thread> for ThreadView {
    fn from(t: Thread) -> Self {
        ThreadView {
            id: t.id.0.raw().to_string(),
            realm_id: t.realm_id.0.raw().to_string(),
            parent_id: t.parent_id.0.raw().to_string(),
            name: t.name,
            owner_id: t.owner_id.map(|o| o.0.raw().to_string()),
            archived: t.archived,
            auto_archive: t.auto_archive,
            message_count: t.message_count,
        }
    }
}

/// 부모 채널 아래 스레드 생성 (CREATE_PUBLIC_THREADS, 채널 컨텍스트) → `THREAD_CREATE` 팬아웃.
async fn create_thread<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(user): AuthUser,
    Path(channel_id): Path<String>,
    Json(req): Json<CreateThreadReq>,
) -> Result<(StatusCode, Json<ThreadView>), ApiError> {
    let parent = parse_channel(&channel_id)?;
    let name = req.name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest("thread name is required".into()));
    }
    let parent_ch = st.store.get(parent).await?.ok_or(ApiError::NotFound)?;
    // 스레드는 텍스트/공지/포럼 채널 아래에만 (스레드 아래 스레드 금지).
    if matches!(parent_ch.kind, domain::channel::ChannelKind::Thread | domain::channel::ChannelKind::Dm) {
        return Err(ApiError::BadRequest("cannot create a thread under this channel".into()));
    }
    crate::perm::require_in_channel(
        &*st.store,
        parent,
        parent_ch.realm_id,
        user,
        Permissions::VIEW_CHANNEL | Permissions::CREATE_PUBLIC_THREADS,
    )
    .await?;

    let id = ChannelId(st.snowflakes.next(st.clock.now_ms()));
    let auto_archive = req.auto_archive.unwrap_or(1440);
    st.store
        .create_thread(&NewThread {
            id,
            realm_id: parent_ch.realm_id,
            parent_id: parent,
            name: name.to_owned(),
            owner: user,
            auto_archive,
        })
        .await?;
    let thread = st.store.get_thread(id).await?.ok_or(ApiError::NotFound)?;

    let _ = st.emitter.emit(parent_ch.realm_id, "THREAD_CREATE".into(), thread_payload(&thread), None).await;
    Ok((StatusCode::CREATED, Json(thread.into())))
}

/// 부모 채널의 스레드 목록 (VIEW_CHANNEL).
async fn list_threads<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(user): AuthUser,
    Path(channel_id): Path<String>,
) -> Result<Json<Vec<ThreadView>>, ApiError> {
    let parent = parse_channel(&channel_id)?;
    let parent_ch = st.store.get(parent).await?.ok_or(ApiError::NotFound)?;
    crate::perm::require_in_channel(&*st.store, parent, parent_ch.realm_id, user, Permissions::VIEW_CHANNEL).await?;
    let threads = st.store.list_threads(parent).await?;
    Ok(Json(threads.into_iter().map(ThreadView::from).collect()))
}

#[derive(Deserialize)]
pub struct UpdateThreadReq {
    pub archived: bool,
}

/// 스레드 아카이브/해제 (소유자 또는 MANAGE_THREADS) → `THREAD_UPDATE` 팬아웃.
async fn update_thread<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(user): AuthUser,
    Path(channel_id): Path<String>,
    Json(req): Json<UpdateThreadReq>,
) -> Result<Json<ThreadView>, ApiError> {
    let tid = parse_channel(&channel_id)?;
    let thread = st.store.get_thread(tid).await?.ok_or(ApiError::NotFound)?;
    // 소유자는 바로, 그 외엔 MANAGE_THREADS(채널 컨텍스트) 필요.
    if thread.owner_id != Some(user) {
        crate::perm::require_in_channel(&*st.store, tid, thread.realm_id, user, Permissions::MANAGE_THREADS).await?;
    } else if !st.store.is_member(thread.realm_id, user).await? {
        return Err(ApiError::Forbidden);
    }
    st.store.set_thread_archived(tid, req.archived).await?;
    let updated = st.store.get_thread(tid).await?.ok_or(ApiError::NotFound)?;

    let _ = st.emitter.emit(thread.realm_id, "THREAD_UPDATE".into(), thread_payload(&updated), None).await;
    Ok(Json(updated.into()))
}
