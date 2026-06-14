//! 인증 에러 (개념: error).

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("password hashing error: {0}")]
    Hash(String),
    #[error("token error: {0}")]
    Token(String),
}
