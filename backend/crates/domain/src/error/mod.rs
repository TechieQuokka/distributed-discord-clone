//! 도메인 에러 (개념: error).

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DomainError {
    #[error("permission denied")]
    PermissionDenied,
    #[error("not found")]
    NotFound,
    #[error("invalid input: {0}")]
    Invalid(String),
}
