//! 인증 라우트 (개념: routes/auth). `/auth/register|login|refresh` (D14/D15).
//!
//! - access 토큰: stateless PASETO v4.public.
//! - refresh 토큰: opaque 랜덤 → 해시만 저장, 회전 + 재사용 탐지.
//! - 비밀번호 해싱/검증(Argon2id)은 CPU 바운드 → `spawn_blocking`.

use auth::{generate_refresh, hash_password, hash_refresh, verify_password};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use domain::id::{RefreshTokenId, UserId};
use domain::refresh_token::NewRefreshToken;
use domain::repo::Store;
use domain::user::NewUser;
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::state::AppState;

/// refresh 토큰 수명 (30일).
const REFRESH_TTL_SECS: i64 = 30 * 24 * 60 * 60;

pub fn routes<S: Store + 'static>() -> axum::Router<AppState<S>> {
    axum::Router::new()
        .route("/auth/register", post(register::<S>))
        .route("/auth/login", post(login::<S>))
        .route("/auth/refresh", post(refresh::<S>))
}

#[derive(Deserialize)]
pub struct RegisterReq {
    pub username: String,
    pub email: String,
    pub password: String,
}

#[derive(Deserialize)]
pub struct LoginReq {
    pub username: String,
    pub password: String,
}

#[derive(Deserialize)]
pub struct RefreshReq {
    pub refresh_token: String,
}

/// id는 Snowflake라 JS 안전정수를 넘으므로 **문자열**로 직렬화 (Discord 관례).
#[derive(Serialize)]
pub struct AuthResponse {
    pub user_id: String,
    pub access_token: String,
    pub refresh_token: String,
}

async fn register<S: Store + 'static>(
    State(st): State<AppState<S>>,
    Json(req): Json<RegisterReq>,
) -> Result<(StatusCode, Json<AuthResponse>), ApiError> {
    let username = req.username.trim();
    let email = req.email.trim();
    if username.is_empty() || email.is_empty() {
        return Err(ApiError::BadRequest("username and email are required".into()));
    }
    if req.password.len() < 8 {
        return Err(ApiError::BadRequest("password must be at least 8 characters".into()));
    }

    let pw = req.password.clone();
    let password_hash = tokio::task::spawn_blocking(move || hash_password(&pw))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))??;

    let id = UserId(st.snowflakes.next(st.clock.now_ms()));
    st.store
        .create_user(&NewUser {
            id,
            username: username.to_owned(),
            email: email.to_owned(),
            password_hash,
        })
        .await?;

    let resp = issue_tokens(&st, id, None).await?;
    Ok((StatusCode::CREATED, Json(resp)))
}

async fn login<S: Store + 'static>(
    State(st): State<AppState<S>>,
    Json(req): Json<LoginReq>,
) -> Result<Json<AuthResponse>, ApiError> {
    let user = st
        .store
        .find_by_username(req.username.trim())
        .await?
        .ok_or(ApiError::Unauthorized)?;

    let phc = user.password_hash.clone();
    let pw = req.password.clone();
    let ok = tokio::task::spawn_blocking(move || verify_password(&pw, &phc))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))??;
    if !ok {
        return Err(ApiError::Unauthorized);
    }

    Ok(Json(issue_tokens(&st, user.id, None).await?))
}

async fn refresh<S: Store + 'static>(
    State(st): State<AppState<S>>,
    Json(req): Json<RefreshReq>,
) -> Result<Json<AuthResponse>, ApiError> {
    let hash = hash_refresh(&req.refresh_token);
    let now_unix = st.now_unix();

    match st.store.find_active(&hash, now_unix).await? {
        Some(active) => {
            st.store.revoke(active.id, now_unix).await?;
            Ok(Json(issue_tokens(&st, active.user_id, Some(active.id)).await?))
        }
        None => {
            // 폐기된 토큰 재제시 = 탈취 의심 → 유저 토큰 체인 전체 무효화 (D14).
            if let Some(seen) = st.store.find_by_hash(&hash).await? {
                st.store.revoke_all_for_user(seen.user_id, now_unix).await?;
            }
            Err(ApiError::Unauthorized)
        }
    }
}

/// access(PASETO) 발급 + refresh(opaque) 생성·저장 → 응답.
async fn issue_tokens<S: Store + 'static>(
    st: &AppState<S>,
    user_id: UserId,
    rotated_from: Option<RefreshTokenId>,
) -> Result<AuthResponse, ApiError> {
    let access_token = st.keys.issue_access(user_id.0.raw())?;
    let (refresh_token, token_hash) = generate_refresh()?;

    let now_ms = st.clock.now_ms();
    let id = RefreshTokenId(st.snowflakes.next(now_ms));
    st.store
        .create_refresh_token(&NewRefreshToken {
            id,
            user_id,
            token_hash,
            rotated_from,
            expires_at_unix: (now_ms / 1000) as i64 + REFRESH_TTL_SECS,
        })
        .await?;

    Ok(AuthResponse {
        user_id: user_id.0.raw().to_string(),
        access_token,
        refresh_token,
    })
}
