//! 메시지 라우트 (개념: routes/message). 히스토리(D38) + 편집/삭제/리액션(D39).
//!
//! 메시지 **전송**은 실시간 경로(Gateway WS)로 처리. 여기선 히스토리 조회 + 사후 변경
//! (편집/소프트삭제/리액션)을 다룬다. 변경은 `RealmEmitter`로 `MESSAGE_*` 팬아웃(D39).

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, put};
use domain::id::{ChannelId, MessageId, RealmId, Snowflake};
use domain::permissions::Permissions;
use domain::repo::Store;
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::events::{message_delete_payload, message_update_payload, reaction_payload};
use crate::extract::AuthUser;
use crate::state::AppState;

pub fn routes<S: Store + 'static>() -> axum::Router<AppState<S>> {
    axum::Router::new()
        .route("/channels/{channel_id}/messages", get(list_messages::<S>))
        .route("/guilds/{guild_id}/messages/search", get(search_messages::<S>))
        .route(
            "/channels/{channel_id}/messages/{message_id}",
            axum::routing::patch(edit_message::<S>).delete(delete_message::<S>),
        )
        .route(
            "/channels/{channel_id}/messages/{message_id}/reactions/{emoji}/@me",
            put(add_reaction::<S>).delete(remove_reaction::<S>),
        )
}

fn parse_channel(s: &str) -> Result<ChannelId, ApiError> {
    s.parse::<u64>().map(|n| ChannelId(Snowflake::from_raw(n))).map_err(|_| ApiError::BadRequest("invalid channel id".into()))
}
fn parse_message(s: &str) -> Result<MessageId, ApiError> {
    s.parse::<u64>().map(|n| MessageId(Snowflake::from_raw(n))).map_err(|_| ApiError::BadRequest("invalid message id".into()))
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

#[derive(Deserialize)]
pub struct SearchQuery {
    /// websearch 구문 (따옴표/OR/- 지원). Postgres `websearch_to_tsquery`가 안전 파싱.
    pub content: String,
    pub limit: Option<i64>,
}

/// 길드 전문검색 (Q10, FTS). 멤버여야 하고, 결과는 **VIEW_CHANNEL 있는 채널**로 한정한다
/// (채널별 권한 존중, D17). 검색 자체는 storage FTS(GIN), 권한 필터는 여기서 적용.
async fn search_messages<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(user): AuthUser,
    Path(guild_id): Path<String>,
    Query(q): Query<SearchQuery>,
) -> Result<Json<Vec<MessageView>>, ApiError> {
    let realm = guild_id
        .parse::<u64>()
        .map(|n| RealmId(Snowflake::from_raw(n)))
        .map_err(|_| ApiError::BadRequest("invalid guild id".into()))?;
    let query = q.content.trim();
    if query.is_empty() {
        return Err(ApiError::BadRequest("empty search query".into()));
    }
    if !st.store.is_member(realm, user).await? {
        return Err(ApiError::Forbidden);
    }

    // VIEW_CHANNEL 있는 채널만 검색 대상 — 채널별 권한 존중(오버라이드 포함).
    let mut allowed = Vec::new();
    for ch in st.store.list_by_realm(realm).await? {
        let perms = crate::perm::effective_in_channel(&*st.store, ch.id, realm, user).await?;
        if perms.contains(Permissions::VIEW_CHANNEL) {
            allowed.push(ch.id);
        }
    }

    let msgs = st.store.search_messages(realm, &allowed, query, q.limit.unwrap_or(25)).await?;
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

#[derive(Deserialize)]
pub struct EditReq {
    pub content: String,
}

/// 메시지 편집 (작성자 본인) → `MESSAGE_UPDATE` 팬아웃.
async fn edit_message<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(user): AuthUser,
    Path((channel_id, message_id)): Path<(String, String)>,
    Json(req): Json<EditReq>,
) -> Result<Json<MessageView>, ApiError> {
    let cid = parse_channel(&channel_id)?;
    let mid = parse_message(&message_id)?;
    let content = req.content.trim();
    if content.is_empty() {
        return Err(ApiError::BadRequest("empty content".into()));
    }

    let msg = st.store.get_message(mid).await?.ok_or(ApiError::NotFound)?;
    if msg.channel_id != cid {
        return Err(ApiError::NotFound);
    }
    if msg.author_id != user {
        return Err(ApiError::Forbidden); // 작성자만 편집.
    }
    if !st.store.edit_message(mid, user, content).await? {
        return Err(ApiError::NotFound); // 경합(이미 삭제 등).
    }

    let payload = message_update_payload(&msg, content);
    let _ = st.emitter.emit(msg.realm_id, "MESSAGE_UPDATE".into(), payload, None).await;
    Ok(Json(MessageView {
        id: mid.0.raw().to_string(),
        channel_id: cid.0.raw().to_string(),
        author_id: user.0.raw().to_string(),
        content: content.to_owned(),
    }))
}

/// 메시지 소프트 삭제 (작성자 본인 또는 MANAGE_MESSAGES) → `MESSAGE_DELETE` 팬아웃.
async fn delete_message<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(user): AuthUser,
    Path((channel_id, message_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let cid = parse_channel(&channel_id)?;
    let mid = parse_message(&message_id)?;

    let msg = st.store.get_message(mid).await?.ok_or(ApiError::NotFound)?;
    if msg.channel_id != cid {
        return Err(ApiError::NotFound);
    }
    // 작성자 본인은 자기 메시지 삭제 가능. 타인 메시지는 MANAGE_MESSAGES(채널 컨텍스트) 필요.
    if msg.author_id != user {
        crate::perm::require_in_channel(&*st.store, cid, msg.realm_id, user, Permissions::MANAGE_MESSAGES).await?;
    }
    if !st.store.soft_delete_message(mid).await? {
        return Err(ApiError::NotFound);
    }

    let payload = message_delete_payload(cid, mid);
    // 이벤트 소싱 사실(D48/E2): 메시지 소프트 삭제 → MessageDeleted. dispatch 단일 소비자가 append.
    let fact = domain::event::RealmEventKind::MessageDeleted { message_id: mid, channel_id: cid };
    let _ = st.emitter.emit(msg.realm_id, "MESSAGE_DELETE".into(), payload, Some(fact)).await;
    Ok(StatusCode::NO_CONTENT)
}

fn validate_emoji(s: &str) -> Result<&str, ApiError> {
    let e = s.trim();
    if e.is_empty() || e.chars().count() > 64 {
        return Err(ApiError::BadRequest("invalid emoji".into()));
    }
    Ok(e)
}

/// 본인 리액션 추가 (ADD_REACTIONS, 채널 컨텍스트) → `MESSAGE_REACTION_ADD` 팬아웃 (멱등).
async fn add_reaction<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(user): AuthUser,
    Path((channel_id, message_id, emoji)): Path<(String, String, String)>,
) -> Result<StatusCode, ApiError> {
    let cid = parse_channel(&channel_id)?;
    let mid = parse_message(&message_id)?;
    let emoji = validate_emoji(&emoji)?;

    let msg = st.store.get_message(mid).await?.ok_or(ApiError::NotFound)?;
    if msg.channel_id != cid {
        return Err(ApiError::NotFound);
    }
    crate::perm::require_in_channel(
        &*st.store,
        cid,
        msg.realm_id,
        user,
        Permissions::VIEW_CHANNEL | Permissions::ADD_REACTIONS,
    )
    .await?;

    // 새로 추가된 경우에만 팬아웃 (멱등 — 중복은 조용히 OK).
    if st.store.add_reaction(mid, user, emoji).await? {
        let payload = reaction_payload(cid, mid, user, emoji);
        let _ = st.emitter.emit(msg.realm_id, "MESSAGE_REACTION_ADD".into(), payload, None).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// 본인 리액션 제거 (멤버) → `MESSAGE_REACTION_REMOVE` 팬아웃.
async fn remove_reaction<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(user): AuthUser,
    Path((channel_id, message_id, emoji)): Path<(String, String, String)>,
) -> Result<StatusCode, ApiError> {
    let cid = parse_channel(&channel_id)?;
    let mid = parse_message(&message_id)?;
    let emoji = validate_emoji(&emoji)?;

    let msg = st.store.get_message(mid).await?.ok_or(ApiError::NotFound)?;
    if msg.channel_id != cid {
        return Err(ApiError::NotFound);
    }
    // 본인 리액션 제거엔 채널 조회 권한이면 충분.
    crate::perm::require_in_channel(&*st.store, cid, msg.realm_id, user, Permissions::VIEW_CHANNEL).await?;

    if st.store.remove_reaction(mid, user, emoji).await? {
        let payload = reaction_payload(cid, mid, user, emoji);
        let _ = st.emitter.emit(msg.realm_id, "MESSAGE_REACTION_REMOVE".into(), payload, None).await;
    }
    Ok(StatusCode::NO_CONTENT)
}
