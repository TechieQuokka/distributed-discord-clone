//! 초대 라우트 (개념: routes/invite). `/guilds/:id/invites`(생성), `/invites/:code`(redeem).
//!
//! 멀티유저 합류의 진입점 (Phase 3). 생성은 멤버만. redeem하면 멤버 1행 추가 →
//! 그 유저가 WS 연결 시 자동 구독(D13)되어 팬아웃 수신 대상이 됨.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::post;
use domain::id::{RealmId, Snowflake};
use domain::invite::NewInvite;
use domain::repo::Store;
use rand::RngExt;
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::extract::AuthUser;
use crate::state::AppState;

pub fn routes<S: Store + 'static>() -> axum::Router<AppState<S>> {
    axum::Router::new()
        .route("/guilds/{realm_id}/invites", post(create_invite::<S>))
        .route("/invites/{code}", post(redeem_invite::<S>))
}

/// 초대 코드 = base62 8자 (CSPRNG). 충돌은 PK 유니크로 방지(드묾).
const CODE_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
fn gen_code() -> String {
    let mut rng = rand::rng();
    (0..8).map(|_| CODE_ALPHABET[rng.random_range(0..CODE_ALPHABET.len())] as char).collect()
}

#[derive(Deserialize, Default)]
pub struct CreateInviteReq {
    /// 0 = 무제한.
    #[serde(default)]
    pub max_uses: i32,
    /// 유효기간(초). 0 = 무기한.
    #[serde(default)]
    pub max_age: i64,
}

#[derive(Serialize)]
pub struct InviteView {
    pub code: String,
    pub realm_id: String,
    pub max_uses: i32,
    pub expires_at: Option<i64>,
}

#[derive(Serialize)]
pub struct JoinView {
    pub realm_id: String,
    pub channels: Vec<ChannelEntry>,
}

#[derive(Serialize)]
pub struct ChannelEntry {
    pub id: String,
    pub name: Option<String>,
}

fn parse_realm(s: &str) -> Result<RealmId, ApiError> {
    s.parse::<u64>()
        .map(|n| RealmId(Snowflake::from_raw(n)))
        .map_err(|_| ApiError::BadRequest("invalid guild id".into()))
}

/// 멤버가 길드 초대 생성.
async fn create_invite<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(user): AuthUser,
    Path(realm_id): Path<String>,
    body: Option<Json<CreateInviteReq>>,
) -> Result<(StatusCode, Json<InviteView>), ApiError> {
    let realm_id = parse_realm(&realm_id)?;
    let req = body.map(|Json(b)| b).unwrap_or_default();
    if req.max_uses < 0 || req.max_age < 0 {
        return Err(ApiError::BadRequest("max_uses/max_age must be >= 0".into()));
    }
    // 초대 생성은 CREATE_INVITE 필요 (@everyone 기본 포함, D17).
    crate::perm::require(&*st.store, realm_id, user, domain::permissions::Permissions::CREATE_INVITE).await?;

    let now_unix = (st.clock.now_ms() / 1000) as i64;
    let expires_at = (req.max_age > 0).then(|| now_unix + req.max_age);
    let code = gen_code();
    st.store
        .create_invite(&NewInvite {
            code: code.clone(),
            realm_id,
            channel_id: None,
            inviter_id: Some(user),
            max_uses: req.max_uses,
            expires_at,
        })
        .await?;

    Ok((
        StatusCode::CREATED,
        Json(InviteView {
            code,
            realm_id: realm_id.0.raw().to_string(),
            max_uses: req.max_uses,
            expires_at,
        }),
    ))
}

/// 초대 코드로 길드 합류. 유효하면 멤버 추가 + 길드 채널 목록 반환.
async fn redeem_invite<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(user): AuthUser,
    Path(code): Path<String>,
) -> Result<Json<JoinView>, ApiError> {
    let now_unix = (st.clock.now_ms() / 1000) as i64;
    let realm_id = st
        .store
        .redeem_invite(&code, user, now_unix)
        .await?
        .ok_or(ApiError::NotFound)?; // 미존재/만료/소진.

    // GUILD_MEMBER_ADD 팬아웃 (D39) — 그 Realm의 현재 접속 멤버들에게 신규 합류 통지.
    // 신규 합류자 본인은 미구독이라 이 응답(JoinView)/다음 READY로 상태 확보(D13).
    if let Some(u) = st.store.find_by_id(user).await? {
        let member = st.store.get_member(realm_id, user).await?;
        let payload = crate::events::member_upsert_payload(realm_id, &u, member.as_ref());
        let _ = st.emitter.emit(realm_id, "GUILD_MEMBER_ADD".into(), payload).await;
    }

    let channels = st
        .store
        .list_by_realm(realm_id)
        .await?
        .into_iter()
        .map(|c| ChannelEntry { id: c.id.0.raw().to_string(), name: c.name })
        .collect();

    Ok(Json(JoinView { realm_id: realm_id.0.raw().to_string(), channels }))
}
