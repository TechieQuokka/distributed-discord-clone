//! 웹훅 라우트 (개념: routes/webhook). Phase 4.
//!
//! 생성/목록/삭제는 MANAGE_WEBHOOKS. **실행**(`POST /webhooks/:id/:token`)은 Bearer 없이 토큰으로 인증 —
//! 채널에 메시지를 게시한다. 토큰은 opaque 랜덤 + SHA-256 해시 저장(auth, D14 철학, P6).
//!
//! ⚠ 실행 시 메시지는 rest-api가 **직접 persist + MESSAGE_CREATE emit**(Realm 액터 우회 seam) — 일반
//! 전송(gateway→actor→dispatch)과 달리 액터 순서화를 거치지 않는다. id는 노드 generator라 단조성 유지.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use domain::id::{ChannelId, MessageId, Snowflake, WebhookId};
use domain::message::NewMessage;
use domain::permissions::Permissions;
use domain::repo::Store;
use domain::webhook::NewWebhook;
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::events::webhook_message_payload;
use crate::extract::AuthUser;
use crate::state::AppState;

pub fn routes<S: Store + 'static>() -> axum::Router<AppState<S>> {
    axum::Router::new()
        .route(
            "/channels/{channel_id}/webhooks",
            get(list_webhooks::<S>).post(create_webhook::<S>),
        )
        .route("/webhooks/{webhook_id}", axum::routing::delete(delete_webhook::<S>))
        .route("/webhooks/{webhook_id}/{token}", post(execute_webhook::<S>))
}

fn parse_channel(s: &str) -> Result<ChannelId, ApiError> {
    s.parse::<u64>().map(|n| ChannelId(Snowflake::from_raw(n))).map_err(|_| ApiError::BadRequest("invalid channel id".into()))
}
fn parse_webhook(s: &str) -> Result<WebhookId, ApiError> {
    s.parse::<u64>().map(|n| WebhookId(Snowflake::from_raw(n))).map_err(|_| ApiError::BadRequest("invalid webhook id".into()))
}

#[derive(Deserialize)]
pub struct CreateWebhookReq {
    pub name: String,
}

#[derive(Serialize)]
pub struct WebhookView {
    pub id: String,
    pub channel_id: String,
    pub name: String,
    /// 생성 시 1회만 반환(이후 조회 불가). 실행 URL `/webhooks/:id/:token`에 사용.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

/// 웹훅 생성 (MANAGE_WEBHOOKS) → 토큰 1회 반환.
async fn create_webhook<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(user): AuthUser,
    Path(channel_id): Path<String>,
    Json(req): Json<CreateWebhookReq>,
) -> Result<(StatusCode, Json<WebhookView>), ApiError> {
    let cid = parse_channel(&channel_id)?;
    let name = req.name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest("webhook name is required".into()));
    }
    let channel = st.store.get(cid).await?.ok_or(ApiError::NotFound)?;
    crate::perm::require(&*st.store, channel.realm_id, user, Permissions::MANAGE_WEBHOOKS).await?;

    let (token, token_hash) = auth::generate_refresh()?;
    let id = WebhookId(st.snowflakes.next(st.clock.now_ms()));
    st.store
        .create_webhook(&NewWebhook {
            id,
            channel_id: cid,
            realm_id: channel.realm_id,
            name: name.to_owned(),
            creator_id: user,
            token_hash,
        })
        .await?;
    crate::routes::audit::record(
        &st, channel.realm_id, user, domain::audit::AuditAction::WebhookCreate, Some(id.0.raw()),
        Some(serde_json::json!({ "name": name }).to_string()),
    )
    .await;

    Ok((
        StatusCode::CREATED,
        Json(WebhookView {
            id: id.0.raw().to_string(),
            channel_id: cid.0.raw().to_string(),
            name: name.to_owned(),
            token: Some(token),
        }),
    ))
}

/// 채널 웹훅 목록 (MANAGE_WEBHOOKS). 토큰은 노출하지 않는다.
async fn list_webhooks<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(user): AuthUser,
    Path(channel_id): Path<String>,
) -> Result<Json<Vec<WebhookView>>, ApiError> {
    let cid = parse_channel(&channel_id)?;
    let channel = st.store.get(cid).await?.ok_or(ApiError::NotFound)?;
    crate::perm::require(&*st.store, channel.realm_id, user, Permissions::MANAGE_WEBHOOKS).await?;

    let list = st.store.list_webhooks(cid).await?;
    Ok(Json(
        list.into_iter()
            .map(|w| WebhookView {
                id: w.id.0.raw().to_string(),
                channel_id: w.channel_id.0.raw().to_string(),
                name: w.name,
                token: None,
            })
            .collect(),
    ))
}

/// 웹훅 삭제 (MANAGE_WEBHOOKS).
async fn delete_webhook<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(user): AuthUser,
    Path(webhook_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let wid = parse_webhook(&webhook_id)?;
    let wh = st.store.get_webhook(wid).await?.ok_or(ApiError::NotFound)?;
    crate::perm::require(&*st.store, wh.realm_id, user, Permissions::MANAGE_WEBHOOKS).await?;
    st.store.delete_webhook(wid).await?;
    crate::routes::audit::record(
        &st, wh.realm_id, user, domain::audit::AuditAction::WebhookDelete, Some(wid.0.raw()), None,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct ExecuteWebhookReq {
    pub content: String,
}

#[derive(Serialize)]
pub struct ExecutedView {
    pub id: String,
    pub channel_id: String,
}

/// 웹훅 실행 (Bearer 없음 — URL 토큰으로 인증). 채널에 메시지 게시 + `MESSAGE_CREATE` 팬아웃.
async fn execute_webhook<S: Store + 'static>(
    State(st): State<AppState<S>>,
    Path((webhook_id, token)): Path<(String, String)>,
    Json(req): Json<ExecuteWebhookReq>,
) -> Result<(StatusCode, Json<ExecutedView>), ApiError> {
    let wid = parse_webhook(&webhook_id)?;
    let content = req.content.trim();
    if content.is_empty() {
        return Err(ApiError::BadRequest("empty content".into()));
    }
    let wh = st.store.get_webhook(wid).await?.ok_or(ApiError::NotFound)?;
    // 토큰 검증: 제시 토큰의 해시 == 저장 해시 (불일치는 401).
    if auth::hash_refresh(&token) != wh.token_hash {
        return Err(ApiError::Unauthorized);
    }
    let author = wh.creator_id.ok_or(ApiError::Internal("webhook has no creator".into()))?;

    // persist-then-emit: 메시지를 직접 적재한 뒤 MESSAGE_CREATE를 Realm 구독자에 팬아웃(액터 우회 seam).
    let mid = MessageId(st.snowflakes.next(st.clock.now_ms()));
    st.store
        .create_message(&NewMessage {
            id: mid,
            channel_id: wh.channel_id,
            realm_id: wh.realm_id,
            author_id: author,
            content: content.to_owned(),
            nonce: None,
            reference_message_id: None,
        })
        .await?;
    let payload = webhook_message_payload(mid, wh.channel_id, author, wid.0.raw(), content);
    // seam(D48/E2): 웹훅 메시지는 Realm 액터를 우회(persist+emit)하므로 이벤트 로그엔 미기록(fact=None).
    // 정식 MessageCreated append는 액터 경로(dispatch MessageCreated 분기)만 — 웹훅 사실화는 후속.
    let _ = st.emitter.emit(wh.realm_id, "MESSAGE_CREATE".into(), payload, None).await;

    Ok((
        StatusCode::OK,
        Json(ExecutedView { id: mid.0.raw().to_string(), channel_id: wh.channel_id.0.raw().to_string() }),
    ))
}
