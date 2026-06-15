//! 첨부 라우트 (개념: routes/attachment). 로컬 FS 첨부 (D37).
//!
//! Discord는 메시지 전송 시 멀티파트로 첨부하지만, 우리 전송 경로는 비동기(gateway→Router→actor라
//! 업로드 시점에 message_id가 없음) → **이미 존재하는 메시지에 사후 첨부**로 단순화(seam).
//! 업로드: `POST /channels/:cid/messages/:mid/attachments`(작성자, ATTACH_FILES) — 바이트는 BlobStore,
//! 메타는 attachments(V14). 다운로드: `GET /attachments/:id`(채널 VIEW_CHANNEL).

use axum::body::Body;
use axum::extract::{Multipart, Path, State};
use axum::http::{StatusCode, header};
use axum::response::Response;
use axum::routing::get;
use axum::Json;
use domain::attachment::NewAttachment;
use domain::id::{AttachmentId, ChannelId, MessageId, Snowflake};
use domain::permissions::Permissions;
use domain::repo::Store;
use serde::Serialize;

use crate::error::ApiError;
use crate::extract::AuthUser;
use crate::state::AppState;

/// 첨부 최대 크기 (8 MiB) — 로컬 study 상한.
const MAX_ATTACHMENT_BYTES: usize = 8 * 1024 * 1024;

pub fn routes<S: Store + 'static>() -> axum::Router<AppState<S>> {
    axum::Router::new()
        .route(
            "/channels/{channel_id}/messages/{message_id}/attachments",
            get(list_attachments::<S>).post(upload_attachment::<S>),
        )
        .route("/attachments/{attachment_id}", get(download_attachment::<S>))
}

fn parse_channel(s: &str) -> Result<ChannelId, ApiError> {
    s.parse::<u64>().map(|n| ChannelId(Snowflake::from_raw(n))).map_err(|_| ApiError::BadRequest("invalid channel id".into()))
}
fn parse_message(s: &str) -> Result<MessageId, ApiError> {
    s.parse::<u64>().map(|n| MessageId(Snowflake::from_raw(n))).map_err(|_| ApiError::BadRequest("invalid message id".into()))
}

#[derive(Serialize)]
pub struct AttachmentView {
    pub id: String,
    pub message_id: String,
    pub filename: String,
    pub size_bytes: i64,
    pub content_type: Option<String>,
    pub url: String,
}

/// 메시지에 파일 첨부 (작성자 본인, VIEW_CHANNEL + ATTACH_FILES). 멀티파트 첫 파일 필드를 저장.
async fn upload_attachment<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(user): AuthUser,
    Path((channel_id, message_id)): Path<(String, String)>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<AttachmentView>), ApiError> {
    let cid = parse_channel(&channel_id)?;
    let mid = parse_message(&message_id)?;

    let msg = st.store.get_message(mid).await?.ok_or(ApiError::NotFound)?;
    if msg.channel_id != cid {
        return Err(ApiError::NotFound);
    }
    if msg.author_id != user {
        return Err(ApiError::Forbidden); // 작성자만 자기 메시지에 첨부.
    }
    crate::perm::require_in_channel(
        &*st.store,
        cid,
        msg.realm_id,
        user,
        Permissions::VIEW_CHANNEL | Permissions::ATTACH_FILES,
    )
    .await?;

    // 멀티파트에서 첫 파일 필드(파일명 보유) 추출.
    let mut file: Option<(String, Option<String>, Vec<u8>)> = None;
    while let Some(field) = multipart.next_field().await.map_err(|e| ApiError::BadRequest(format!("multipart: {e}")))? {
        if let Some(fname) = field.file_name().map(|s| s.to_owned()) {
            let content_type = field.content_type().map(|s| s.to_owned());
            let bytes = field.bytes().await.map_err(|e| ApiError::BadRequest(format!("read field: {e}")))?;
            if bytes.len() > MAX_ATTACHMENT_BYTES {
                return Err(ApiError::BadRequest("attachment too large (max 8 MiB)".into()));
            }
            file = Some((fname, content_type, bytes.to_vec()));
            break;
        }
    }
    let (filename, content_type, bytes) = file.ok_or(ApiError::BadRequest("no file field in multipart".into()))?;
    let size_bytes = bytes.len() as i64;

    let id = AttachmentId(st.snowflakes.next(st.clock.now_ms()));
    let key = id.0.raw().to_string();
    // 바이트 먼저 저장(BlobStore) → 메타 적재. 메타 적재 실패해도 고아 블롭은 무해(미참조).
    st.blobs.put(&key, bytes).await.map_err(|e| ApiError::Internal(e.to_string()))?;
    let url = format!("/attachments/{key}");
    st.store
        .add_attachment(&NewAttachment {
            id,
            message_id: mid,
            filename: sanitize_filename(&filename),
            size_bytes,
            content_type: content_type.clone(),
            url: url.clone(),
        })
        .await?;

    Ok((
        StatusCode::CREATED,
        Json(AttachmentView {
            id: key,
            message_id: mid.0.raw().to_string(),
            filename: sanitize_filename(&filename),
            size_bytes,
            content_type,
            url,
        }),
    ))
}

/// 메시지의 첨부 목록 (VIEW_CHANNEL).
async fn list_attachments<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(user): AuthUser,
    Path((channel_id, message_id)): Path<(String, String)>,
) -> Result<Json<Vec<AttachmentView>>, ApiError> {
    let cid = parse_channel(&channel_id)?;
    let mid = parse_message(&message_id)?;
    let msg = st.store.get_message(mid).await?.ok_or(ApiError::NotFound)?;
    if msg.channel_id != cid {
        return Err(ApiError::NotFound);
    }
    crate::perm::require_in_channel(&*st.store, cid, msg.realm_id, user, Permissions::VIEW_CHANNEL).await?;

    let list = st.store.list_attachments(mid).await?;
    Ok(Json(
        list.into_iter()
            .map(|a| AttachmentView {
                id: a.id.0.raw().to_string(),
                message_id: a.message_id.0.raw().to_string(),
                filename: a.filename,
                size_bytes: a.size_bytes,
                content_type: a.content_type,
                url: a.url,
            })
            .collect(),
    ))
}

/// 첨부 다운로드 (그 메시지 채널의 VIEW_CHANNEL 필요). 바이트를 content-type과 함께 반환.
async fn download_attachment<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(user): AuthUser,
    Path(attachment_id): Path<String>,
) -> Result<Response, ApiError> {
    let aid = AttachmentId(Snowflake::from_raw(
        attachment_id.parse::<u64>().map_err(|_| ApiError::BadRequest("invalid attachment id".into()))?,
    ));
    let att = st.store.get_attachment(aid).await?.ok_or(ApiError::NotFound)?;
    // 권한: 첨부 → 메시지 → 채널의 VIEW_CHANNEL.
    let msg = st.store.get_message(att.message_id).await?.ok_or(ApiError::NotFound)?;
    crate::perm::require_in_channel(&*st.store, msg.channel_id, msg.realm_id, user, Permissions::VIEW_CHANNEL).await?;

    let bytes = st
        .blobs
        .get(&aid.0.raw().to_string())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;

    let ct = att.content_type.unwrap_or_else(|| "application/octet-stream".into());
    let disposition = format!("attachment; filename=\"{}\"", att.filename.replace('"', ""));
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, ct)
        .header(header::CONTENT_DISPOSITION, disposition)
        .body(Body::from(bytes))
        .map_err(|e| ApiError::Internal(e.to_string()))?)
}

/// 파일명에서 경로 구분자 제거 (저장은 id 키지만 메타/다운로드 헤더 안전용).
fn sanitize_filename(name: &str) -> String {
    name.rsplit(['/', '\\']).next().unwrap_or(name).chars().take(255).collect()
}
