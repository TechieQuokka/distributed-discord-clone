//! 바이너리 블롭 저장 port (개념: blob). 첨부 파일 바이트(D37).
//!
//! domain은 IO를 모른다(P2) — 실제 저장(로컬 FS, 후속 MinIO/S3)은 adapter가 구현.
//! 메타데이터(파일명/크기)는 `attachments` 테이블(AttachmentRepository), 바이트만 여기로.
//! repo 포트(RPITIT)와 달리 `dyn` 주입이 필요해(엣지가 구현을 모름) 박스 future를 쓴다(`emit`과 동형).

use crate::emit::BoxFuture;

#[derive(Debug, thiserror::Error)]
pub enum BlobError {
    #[error("blob backend error: {0}")]
    Backend(String),
}

/// 블롭 저장소 port. `key` = 첨부 id 등 불투명 키.
pub trait BlobStore: Send + Sync {
    /// 바이트 저장(덮어쓰기). 키는 호출측이 유일하게 만든다(예: 첨부 Snowflake id).
    fn put(&self, key: &str, bytes: Vec<u8>) -> BoxFuture<'_, Result<(), BlobError>>;

    /// 바이트 조회. 없으면 `Ok(None)`.
    fn get(&self, key: &str) -> BoxFuture<'_, Result<Option<Vec<u8>>, BlobError>>;
}
