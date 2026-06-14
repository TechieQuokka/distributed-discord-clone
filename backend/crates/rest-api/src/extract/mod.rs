//! 추출기 (개념: extract). `Authorization: Bearer <PASETO>` → 인증된 유저 (D14).

use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use domain::id::{Snowflake, UserId};
use domain::repo::Store;

use crate::error::ApiError;
use crate::state::AppState;

/// 검증된 access 토큰의 소유자.
pub struct AuthUser(pub UserId);

impl<S: Store + 'static> FromRequestParts<AppState<S>> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState<S>,
    ) -> Result<Self, Self::Rejection> {
        let token = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.strip_prefix("Bearer "))
            .ok_or(ApiError::Unauthorized)?;
        let uid = state.keys.verify_access(token).map_err(|_| ApiError::Unauthorized)?;
        Ok(AuthUser(UserId(Snowflake::from_raw(uid))))
    }
}
