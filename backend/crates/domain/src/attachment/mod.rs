//! 첨부 엔티티 (개념: attachment). 순수 데이터 — IO 무의존. 스키마 02-schema.md §5 `attachments` (D37).
//!
//! 메타데이터(파일명/크기/타입/url)는 `attachments` 테이블에, 실제 바이트는 [`crate::blob::BlobStore`]
//! (로컬 FS 등) 뒤에 둔다 — domain은 둘 다 port로만 안다 (P2).

use crate::id::{AttachmentId, MessageId};

/// 저장된 첨부 메타데이터.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    pub id: AttachmentId,
    pub message_id: MessageId,
    pub filename: String,
    pub size_bytes: i64,
    pub content_type: Option<String>,
    /// 다운로드 경로(예: `/attachments/<id>`). 바이트는 BlobStore가 보관.
    pub url: String,
}

/// 신규 첨부 입력.
#[derive(Clone, Debug)]
pub struct NewAttachment {
    pub id: AttachmentId,
    pub message_id: MessageId,
    pub filename: String,
    pub size_bytes: i64,
    pub content_type: Option<String>,
    pub url: String,
}
