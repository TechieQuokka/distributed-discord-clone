//! WebAuthn/Passkeys 라우트 (개념: routes/webauthn). D19. register/login ceremony(start·finish).
//!
//! 크립토/검증은 `auth::WebauthnService`(webauthn-rs, P6). ceremony 중간 상태는 `AppState.ceremonies`
//! (휘발, DB-D5)에 ceremony_id로 보관. 자격증명(Passkey)은 직렬화해 `webauthn_credentials`(V18)에.
//! webauthn 미설정 노드는 404(`require_webauthn`).

use auth::webauthn::{Passkey, PublicKeyCredential, RegisterPublicKeyCredential};
use auth::{WebauthnService, cred_id_bytes, user_uuid};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use domain::id::UserId;
use domain::repo::Store;
use domain::webauthn::NewWebAuthnCredential;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::error::ApiError;
use crate::extract::AuthUser;
use crate::routes::auth::issue_tokens;
use crate::state::{AppState, Ceremony};

/// ceremony 유효시간(ms).
const CEREMONY_TTL_MS: u64 = 5 * 60 * 1000;

pub fn routes<S: Store + 'static>() -> axum::Router<AppState<S>> {
    axum::Router::new()
        .route("/auth/webauthn/register/start", post(register_start::<S>))
        .route("/auth/webauthn/register/finish", post(register_finish::<S>))
        .route("/auth/webauthn/login/start", post(login_start::<S>))
        .route("/auth/webauthn/login/finish", post(login_finish::<S>))
        // Usernameless(discoverable) 로그인 (D19) — username 없이.
        .route("/auth/webauthn/login/discoverable/start", post(discoverable_start::<S>))
        .route("/auth/webauthn/login/discoverable/finish", post(discoverable_finish::<S>))
}

/// webauthn 미설정이면 404. 설정됐으면 서비스 반환.
fn require_webauthn<S: Store>(st: &AppState<S>) -> Result<Arc<WebauthnService>, ApiError> {
    st.webauthn.clone().ok_or(ApiError::NotFound)
}

fn load_passkeys(jsons: &[String]) -> Vec<Passkey> {
    jsons.iter().filter_map(|j| serde_json::from_str::<Passkey>(j).ok()).collect()
}

// ─── 등록 (인증된 유저) ──────────────────────────────────────────────────────

async fn register_start<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(uid): AuthUser,
) -> Result<Json<Value>, ApiError> {
    let svc = require_webauthn(&st)?;
    let user = st.store.find_by_id(uid).await?.ok_or(ApiError::Unauthorized)?;
    let display = user.global_name.clone().unwrap_or_else(|| user.username.clone());
    let existing: Vec<String> =
        st.store.list_credentials(uid).await?.into_iter().map(|c| c.passkey_json).collect();
    let exclude = load_passkeys(&existing);

    let (ccr, reg_state) = svc
        .start_register(user_uuid(uid.0.raw()), &user.username, &display, &exclude)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let cid = st.snowflakes.next(st.clock.now_ms()).raw();
    let expiry = st.clock.now_ms() + CEREMONY_TTL_MS;
    st.ceremonies
        .lock()
        .unwrap()
        .insert(cid, (Ceremony::Register { user_id: uid.0.raw(), state: Box::new(reg_state) }, expiry));
    Ok(Json(json!({ "ceremony_id": cid.to_string(), "options": ccr })))
}

#[derive(Deserialize)]
struct RegisterFinishReq {
    ceremony_id: String,
    credential: RegisterPublicKeyCredential,
}

async fn register_finish<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(uid): AuthUser,
    Json(req): Json<RegisterFinishReq>,
) -> Result<StatusCode, ApiError> {
    let svc = require_webauthn(&st)?;
    let cid: u64 = req.ceremony_id.parse().map_err(|_| ApiError::BadRequest("bad ceremony_id".into()))?;
    let state = take_ceremony(&st, cid)?;
    let Ceremony::Register { user_id, state } = state else {
        return Err(ApiError::BadRequest("ceremony type mismatch".into()));
    };
    if user_id != uid.0.raw() {
        return Err(ApiError::Forbidden);
    }
    let passkey = svc
        .finish_register(&req.credential, &state)
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    let json = serde_json::to_string(&passkey).map_err(|e| ApiError::Internal(e.to_string()))?;
    st.store
        .add_credential(&NewWebAuthnCredential {
            id: st.snowflakes.next(st.clock.now_ms()).raw(),
            user_id: uid,
            credential_id: cred_id_bytes(&passkey),
            passkey_json: json,
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ─── 로그인 (암호 없는) ──────────────────────────────────────────────────────

#[derive(Deserialize)]
struct LoginStartReq {
    username: String,
}

async fn login_start<S: Store + 'static>(
    State(st): State<AppState<S>>,
    Json(req): Json<LoginStartReq>,
) -> Result<Json<Value>, ApiError> {
    let svc = require_webauthn(&st)?;
    let user = st.store.find_by_username(req.username.trim()).await?.ok_or(ApiError::Unauthorized)?;
    let creds: Vec<String> =
        st.store.list_credentials(user.id).await?.into_iter().map(|c| c.passkey_json).collect();
    let passkeys = load_passkeys(&creds);
    if passkeys.is_empty() {
        return Err(ApiError::Unauthorized); // 등록된 passkey 없음.
    }
    let (rcr, auth_state) =
        svc.start_auth(&passkeys).map_err(|e| ApiError::Internal(e.to_string()))?;

    let cid = st.snowflakes.next(st.clock.now_ms()).raw();
    let expiry = st.clock.now_ms() + CEREMONY_TTL_MS;
    st.ceremonies
        .lock()
        .unwrap()
        .insert(cid, (Ceremony::Auth { user_id: user.id.0.raw(), state: Box::new(auth_state) }, expiry));
    Ok(Json(json!({ "ceremony_id": cid.to_string(), "options": rcr })))
}

#[derive(Deserialize)]
struct LoginFinishReq {
    ceremony_id: String,
    credential: PublicKeyCredential,
}

async fn login_finish<S: Store + 'static>(
    State(st): State<AppState<S>>,
    Json(req): Json<LoginFinishReq>,
) -> Result<Json<Value>, ApiError> {
    let svc = require_webauthn(&st)?;
    let cid: u64 = req.ceremony_id.parse().map_err(|_| ApiError::BadRequest("bad ceremony_id".into()))?;
    let state = take_ceremony(&st, cid)?;
    let Ceremony::Auth { user_id, state } = state else {
        return Err(ApiError::BadRequest("ceremony type mismatch".into()));
    };
    let res = svc.finish_auth(&req.credential, &state).map_err(|_| ApiError::Unauthorized)?;

    // counter 진전 시 저장된 passkey 갱신(클론 탐지 토대).
    if res.needs_update() {
        let cred_bytes: Vec<u8> = res.cred_id().as_ref().to_vec();
        let uid = UserId(domain::id::Snowflake::from_raw(user_id));
        for c in st.store.list_credentials(uid).await? {
            if c.credential_id == cred_bytes {
                if let Ok(mut pk) = serde_json::from_str::<Passkey>(&c.passkey_json) {
                    if pk.update_credential(&res).is_some() {
                        if let Ok(j) = serde_json::to_string(&pk) {
                            st.store.update_credential(&cred_bytes, &j).await?;
                        }
                    }
                }
            }
        }
    }

    let uid = UserId(domain::id::Snowflake::from_raw(user_id));
    let resp = issue_tokens(&st, uid, None).await?;
    Ok(Json(serde_json::to_value(resp).unwrap_or(Value::Null)))
}

// ─── Usernameless (discoverable) 로그인 (D19) ────────────────────────────────
// username 없이: 인증기가 resident key로 유저를 고른다. start는 누구인지 모른 채 challenge만 발급,
// finish에서 자격증명의 user handle로 유저 식별 → 그 유저 passkey로 검증.

async fn discoverable_start<S: Store + 'static>(
    State(st): State<AppState<S>>,
) -> Result<Json<Value>, ApiError> {
    let svc = require_webauthn(&st)?;
    let (rcr, state) =
        svc.start_discoverable_auth().map_err(|e| ApiError::Internal(e.to_string()))?;
    let cid = st.snowflakes.next(st.clock.now_ms()).raw();
    let expiry = st.clock.now_ms() + CEREMONY_TTL_MS;
    st.ceremonies
        .lock()
        .unwrap()
        .insert(cid, (Ceremony::Discoverable { state: Box::new(state) }, expiry));
    Ok(Json(json!({ "ceremony_id": cid.to_string(), "options": rcr })))
}

async fn discoverable_finish<S: Store + 'static>(
    State(st): State<AppState<S>>,
    Json(req): Json<LoginFinishReq>,
) -> Result<Json<Value>, ApiError> {
    let svc = require_webauthn(&st)?;
    let cid: u64 = req.ceremony_id.parse().map_err(|_| ApiError::BadRequest("bad ceremony_id".into()))?;
    let state = take_ceremony(&st, cid)?;
    let Ceremony::Discoverable { state } = state else {
        return Err(ApiError::BadRequest("ceremony type mismatch".into()));
    };
    // 자격증명의 user handle → 유저 Snowflake 식별 → 그 유저 passkey 로드.
    let snowflake = svc.identify_discoverable(&req.credential).map_err(|_| ApiError::Unauthorized)?;
    let uid = UserId(domain::id::Snowflake::from_raw(snowflake));
    let creds: Vec<String> =
        st.store.list_credentials(uid).await?.into_iter().map(|c| c.passkey_json).collect();
    let passkeys = load_passkeys(&creds);
    if passkeys.is_empty() {
        return Err(ApiError::Unauthorized);
    }
    let res = svc
        .finish_discoverable_auth(&req.credential, *state, &passkeys)
        .map_err(|_| ApiError::Unauthorized)?;

    // counter 진전 시 갱신(클론 탐지).
    if res.needs_update() {
        let cred_bytes: Vec<u8> = res.cred_id().as_ref().to_vec();
        for c in st.store.list_credentials(uid).await? {
            if c.credential_id == cred_bytes {
                if let Ok(mut pk) = serde_json::from_str::<Passkey>(&c.passkey_json) {
                    if pk.update_credential(&res).is_some() {
                        if let Ok(j) = serde_json::to_string(&pk) {
                            st.store.update_credential(&cred_bytes, &j).await?;
                        }
                    }
                }
            }
        }
    }

    let resp = issue_tokens(&st, uid, None).await?;
    Ok(Json(serde_json::to_value(resp).unwrap_or(Value::Null)))
}

/// ceremony 꺼내기(제거 + 만료 검사).
fn take_ceremony<S: Store>(st: &AppState<S>, cid: u64) -> Result<Ceremony, ApiError> {
    let (cer, expiry) = st
        .ceremonies
        .lock()
        .unwrap()
        .remove(&cid)
        .ok_or(ApiError::BadRequest("unknown or used ceremony".into()))?;
    if st.clock.now_ms() > expiry {
        return Err(ApiError::BadRequest("ceremony expired".into()));
    }
    Ok(cer)
}
