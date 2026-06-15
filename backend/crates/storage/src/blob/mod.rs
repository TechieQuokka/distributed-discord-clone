//! 로컬 FS 블롭 저장소 (개념: blob). `BlobStore` port의 adapter (D37).
//!
//! 첨부 바이트를 `<base>/<key>` 파일로 저장/조회. MinIO/S3 업그레이드는 같은 port의 다른 adapter(D37).
//! key는 호출측이 유일 키(첨부 Snowflake id)로 주므로 경로 조작 방지를 위해 영숫자만 허용한다.

use std::path::PathBuf;

use domain::blob::{BlobError, BlobStore};
use domain::emit::BoxFuture;

#[derive(Clone)]
pub struct LocalFsBlobStore {
    base: PathBuf,
}

impl LocalFsBlobStore {
    /// `base` 디렉터리를 만들고(없으면) 준비. 첨부 루트.
    pub fn new(base: impl Into<PathBuf>) -> std::io::Result<Self> {
        let base = base.into();
        std::fs::create_dir_all(&base)?;
        Ok(Self { base })
    }

    /// key 유효성: 영숫자만(경로 탈출 `..`/`/` 차단). 첨부 id(숫자)면 항상 통과.
    fn safe_path(&self, key: &str) -> Result<PathBuf, BlobError> {
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(BlobError::Backend(format!("invalid blob key: {key}")));
        }
        Ok(self.base.join(key))
    }
}

impl BlobStore for LocalFsBlobStore {
    fn put(&self, key: &str, bytes: Vec<u8>) -> BoxFuture<'_, Result<(), BlobError>> {
        let path = self.safe_path(key);
        Box::pin(async move {
            let path = path?;
            tokio::fs::write(&path, &bytes).await.map_err(|e| BlobError::Backend(e.to_string()))
        })
    }

    fn get(&self, key: &str) -> BoxFuture<'_, Result<Option<Vec<u8>>, BlobError>> {
        let path = self.safe_path(key);
        Box::pin(async move {
            let path = path?;
            match tokio::fs::read(&path).await {
                Ok(b) => Ok(Some(b)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(BlobError::Backend(e.to_string())),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn put_get_roundtrip_and_safe_key() {
        let dir = std::env::temp_dir().join(format!("blobtest_{}", std::process::id()));
        let s = LocalFsBlobStore::new(&dir).unwrap();

        s.put("123", b"hello bytes".to_vec()).await.unwrap();
        assert_eq!(s.get("123").await.unwrap().as_deref(), Some(&b"hello bytes"[..]));
        assert_eq!(s.get("999").await.unwrap(), None, "없는 키는 None");

        // 경로 탈출 키는 거부.
        assert!(s.put("../escape", b"x".to_vec()).await.is_err());
        assert!(s.get("a/b").await.is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
