//! 인증 라우트 (개념: routes/auth). `/auth/register|login|refresh` (D14/D15).
//!
//! - access 토큰: stateless PASETO v4.public.
//! - refresh 토큰: opaque 랜덤 → 해시만 저장, 회전 + 재사용 탐지.
//! - 비밀번호 해싱/검증(Argon2id)은 CPU 바운드 → `spawn_blocking`.

use auth::pow::DEFAULT_DIFFICULTY;
use auth::{generate_refresh, hash_password, hash_refresh, totp, verify_password};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use domain::id::{RefreshTokenId, UserId};
use domain::refresh_token::NewRefreshToken;
use domain::repo::Store;
use domain::user::NewUser;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::ApiError;
use crate::extract::AuthUser;
use crate::state::AppState;

/// refresh 토큰 수명 (30일).
const REFRESH_TTL_SECS: i64 = 30 * 24 * 60 * 60;

pub fn routes<S: Store + 'static>() -> axum::Router<AppState<S>> {
    axum::Router::new()
        .route("/auth/pow-challenge", get(pow_challenge::<S>))
        .route("/auth/register", post(register::<S>))
        .route("/auth/login", post(login::<S>))
        .route("/auth/refresh", post(refresh::<S>))
        // TOTP MFA (D19): enable(발급)→verify(확인 시 저장=활성)→login 2단계(코드 재제출).
        .route("/auth/mfa/totp/enable", post(mfa_enable::<S>))
        .route("/auth/mfa/totp/verify", post(mfa_verify::<S>))
        .route("/auth/mfa/totp/disable", post(mfa_disable::<S>))
        .route("/auth/mfa/totp", post(mfa_login::<S>))
}

/// 가입용 PoW 챌린지 응답 (D18). `challenge`=PASETO v4.local 토큰, `difficulty`=선행 0비트.
#[derive(Serialize)]
pub struct PowChallengeResp {
    pub challenge: String,
    pub difficulty: u8,
}

#[derive(Deserialize)]
pub struct RegisterReq {
    pub username: String,
    pub email: String,
    pub password: String,
    /// 봇방지 PoW (D18): pow-challenge에서 받은 토큰 + 해를 만족하는 nonce.
    pub pow_challenge: String,
    pub pow_nonce: String,
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
pub(crate) struct AuthResponse {
    pub user_id: String,
    pub access_token: String,
    pub refresh_token: String,
}

/// 가입용 PoW 챌린지 발급 (D18). stateless — 서버는 챌린지를 저장하지 않는다(DB-D5).
async fn pow_challenge<S: Store + 'static>(
    State(st): State<AppState<S>>,
) -> Result<Json<PowChallengeResp>, ApiError> {
    let challenge = st
        .pow
        .issue_challenge(DEFAULT_DIFFICULTY)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(PowChallengeResp { challenge, difficulty: DEFAULT_DIFFICULTY }))
}

async fn register<S: Store + 'static>(
    State(st): State<AppState<S>>,
    Json(req): Json<RegisterReq>,
) -> Result<(StatusCode, Json<AuthResponse>), ApiError> {
    // 봇방지 게이트 (D18): 유효한 PoW 해가 없으면 가입 거부.
    st.pow
        .verify(&req.pow_challenge, &req.pow_nonce)
        .map_err(|_| ApiError::BadRequest("invalid or expired proof-of-work".into()))?;

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

/// 로그인. MFA(D19) 활성 유저면 토큰 대신 `{ "mfa_required": true }` → `POST /auth/mfa/totp`로 2단계.
async fn login<S: Store + 'static>(
    State(st): State<AppState<S>>,
    Json(req): Json<LoginReq>,
) -> Result<Json<Value>, ApiError> {
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

    // MFA 게이트: secret이 있으면 비번만으론 토큰 발급 안 함.
    if st.store.totp_secret(user.id).await?.is_some() {
        return Ok(Json(json!({ "mfa_required": true })));
    }
    let resp = issue_tokens(&st, user.id, None).await?;
    Ok(Json(serde_json::to_value(resp).unwrap_or(Value::Null)))
}

/// MFA enable (인증 필요): TOTP secret 발급 + `otpauth://` URI 반환. **아직 저장 안 함** —
/// verify가 코드로 확인해야 저장(활성). 락아웃 방지(미확인 secret로 잠기지 않음).
#[derive(Serialize)]
struct MfaEnableResp {
    secret: String, // hex (verify에 다시 제출)
    otpauth_uri: String,
}

async fn mfa_enable<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(uid): AuthUser,
) -> Result<Json<MfaEnableResp>, ApiError> {
    let user = st.store.find_by_id(uid).await?.ok_or(ApiError::Unauthorized)?;
    let secret = totp::new_secret();
    let uri = totp::otpauth_uri(&secret, &user.username).map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(MfaEnableResp { secret: totp::encode_hex(&secret), otpauth_uri: uri }))
}

#[derive(Deserialize)]
struct MfaVerifyReq {
    secret: String, // enable에서 받은 hex
    code: String,
}

/// MFA verify (인증 필요): secret+code가 맞으면 **저장 → 활성화**.
async fn mfa_verify<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(uid): AuthUser,
    Json(req): Json<MfaVerifyReq>,
) -> Result<StatusCode, ApiError> {
    let secret = totp::decode_hex(&req.secret).map_err(|_| ApiError::BadRequest("bad secret".into()))?;
    let now = st.now_unix() as u64;
    let ok = totp::verify(&secret, &req.code, now).map_err(|_| ApiError::BadRequest("totp error".into()))?;
    if !ok {
        return Err(ApiError::BadRequest("invalid code".into()));
    }
    st.store.set_totp_secret(uid, Some(&secret)).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct MfaCodeReq {
    code: String,
}

/// MFA disable (인증 필요): 현재 코드 확인 후 secret 제거.
async fn mfa_disable<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(uid): AuthUser,
    Json(req): Json<MfaCodeReq>,
) -> Result<StatusCode, ApiError> {
    let secret = st.store.totp_secret(uid).await?.ok_or(ApiError::BadRequest("mfa not enabled".into()))?;
    let now = st.now_unix() as u64;
    if !totp::verify(&secret, &req.code, now).unwrap_or(false) {
        return Err(ApiError::BadRequest("invalid code".into()));
    }
    st.store.set_totp_secret(uid, None).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct MfaLoginReq {
    username: String,
    password: String,
    code: String,
}

/// 로그인 2단계 (D19): 비번 재확인 + TOTP 코드 검증 → 토큰 발급.
async fn mfa_login<S: Store + 'static>(
    State(st): State<AppState<S>>,
    Json(req): Json<MfaLoginReq>,
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

    let secret = st.store.totp_secret(user.id).await?.ok_or(ApiError::BadRequest("mfa not enabled".into()))?;
    let now = st.now_unix() as u64;
    if !totp::verify(&secret, &req.code, now).unwrap_or(false) {
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

/// access(PASETO) 발급 + refresh(opaque) 생성·저장 → 응답. (webauthn 로그인 등에서도 재사용)
pub(crate) async fn issue_tokens<S: Store + 'static>(
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
