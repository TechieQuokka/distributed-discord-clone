//! REST 에러 → HTTP 응답 매핑 (개념: error).

use auth::AuthError;
use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use domain::repo::RepoError;
use serde::Serialize;

#[derive(Debug)]
pub enum ApiError {
    BadRequest(String),
    Unauthorized,
    Forbidden,
    NotFound,
    Conflict(String),
    Internal(String),
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
    message: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, msg) = match self {
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, "bad_request", m),
            ApiError::Unauthorized => {
                (StatusCode::UNAUTHORIZED, "unauthorized", "invalid credentials".into())
            }
            ApiError::Forbidden => {
                (StatusCode::FORBIDDEN, "forbidden", "not permitted".into())
            }
            ApiError::NotFound => (StatusCode::NOT_FOUND, "not_found", "not found".into()),
            ApiError::Conflict(m) => (StatusCode::CONFLICT, "conflict", m),
            ApiError::Internal(m) => {
                // 내부 메시지는 로그로, 클라엔 일반화된 문구.
                tracing::error!(error = %m, "internal error");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal", "internal server error".into())
            }
        };
        (status, Json(ErrorBody { error: code.into(), message: msg })).into_response()
    }
}

impl From<RepoError> for ApiError {
    fn from(e: RepoError) -> Self {
        match e {
            RepoError::Conflict => ApiError::Conflict("resource already exists".into()),
            RepoError::Backend(m) => ApiError::Internal(m),
        }
    }
}

impl From<AuthError> for ApiError {
    fn from(e: AuthError) -> Self {
        ApiError::Internal(e.to_string())
    }
}
