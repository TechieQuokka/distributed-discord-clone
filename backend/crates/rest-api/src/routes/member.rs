//! 멤버 관리 라우트 (개념: routes/member). 조회/nick수정/추방·탈퇴 (D39).
//!
//! 조회=멤버. nick=본인 `CHANGE_NICKNAME` 또는 타인 `MANAGE_NICKNAMES`. 추방=`KICK_MEMBERS`(본인=탈퇴).
//! 변동은 `RealmEmitter`로 `GUILD_MEMBER_UPDATE`/`_REMOVE` 팬아웃 → 그 Realm 현재 접속 구독자에 통지.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use domain::id::{RealmId, Snowflake, UserId};
use domain::member::Member;
use domain::permissions::Permissions;
use domain::repo::Store;
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::events::{member_remove_payload, member_upsert_payload};
use crate::extract::AuthUser;
use crate::perm;
use crate::state::AppState;

pub fn routes<S: Store + 'static>() -> axum::Router<AppState<S>> {
    axum::Router::new()
        .route("/guilds/{realm_id}/members", get(list_members::<S>))
        .route(
            "/guilds/{realm_id}/members/{user_id}",
            get(get_member::<S>).patch(patch_member::<S>).delete(remove_member::<S>),
        )
}

fn parse_realm(s: &str) -> Result<RealmId, ApiError> {
    s.parse::<u64>().map(|n| RealmId(Snowflake::from_raw(n))).map_err(|_| ApiError::BadRequest("invalid guild id".into()))
}
/// 경로의 user_id를 해석 — `@me`는 호출자 자신(Discord 관례).
fn resolve_user(s: &str, me: UserId) -> Result<UserId, ApiError> {
    if s == "@me" {
        return Ok(me);
    }
    s.parse::<u64>().map(|n| UserId(Snowflake::from_raw(n))).map_err(|_| ApiError::BadRequest("invalid user id".into()))
}

#[derive(Serialize)]
pub struct MemberView {
    pub user_id: String,
    pub nick: Option<String>,
    pub joined_at: i64,
    pub roles: Vec<String>,
}

impl From<Member> for MemberView {
    fn from(m: Member) -> Self {
        MemberView {
            user_id: m.user_id.0.raw().to_string(),
            nick: m.nick,
            joined_at: m.joined_at,
            roles: m.roles.iter().map(|r| r.0.raw().to_string()).collect(),
        }
    }
}

/// 멤버 목록 (멤버만).
async fn list_members<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(user): AuthUser,
    Path(realm_id): Path<String>,
) -> Result<Json<Vec<MemberView>>, ApiError> {
    let realm = parse_realm(&realm_id)?;
    if !st.store.is_member(realm, user).await? {
        return Err(ApiError::Forbidden);
    }
    let members = st.store.list_members(realm).await?;
    Ok(Json(members.into_iter().map(MemberView::from).collect()))
}

/// 멤버 단건 조회 (멤버만).
async fn get_member<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(user): AuthUser,
    Path((realm_id, user_id)): Path<(String, String)>,
) -> Result<Json<MemberView>, ApiError> {
    let realm = parse_realm(&realm_id)?;
    let target = resolve_user(&user_id, user)?;
    if !st.store.is_member(realm, user).await? {
        return Err(ApiError::Forbidden);
    }
    let m = st.store.get_member(realm, target).await?.ok_or(ApiError::NotFound)?;
    Ok(Json(m.into()))
}

#[derive(Deserialize, Default)]
pub struct PatchMemberReq {
    /// nick 설정. 생략/`null` = 제거.
    #[serde(default)]
    pub nick: Option<String>,
}

/// 멤버 nick 수정 → `GUILD_MEMBER_UPDATE` 팬아웃.
async fn patch_member<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(actor): AuthUser,
    Path((realm_id, user_id)): Path<(String, String)>,
    body: Option<Json<PatchMemberReq>>,
) -> Result<Json<MemberView>, ApiError> {
    let realm = parse_realm(&realm_id)?;
    let target = resolve_user(&user_id, actor)?;
    let req = body.map(|Json(b)| b).unwrap_or_default();

    // 본인 닉은 CHANGE_NICKNAME, 타인 닉은 MANAGE_NICKNAMES (둘 다 멤버십 포함, perm::require).
    let needed =
        if target == actor { Permissions::CHANGE_NICKNAME } else { Permissions::MANAGE_NICKNAMES };
    perm::require(&*st.store, realm, actor, needed).await?;

    // nick은 trim, 빈 문자열은 제거로 취급.
    let nick = req.nick.as_deref().map(str::trim).filter(|s| !s.is_empty());
    if !st.store.update_member_nick(realm, target, nick).await? {
        return Err(ApiError::NotFound); // 대상이 멤버 아님.
    }

    let member = st.store.get_member(realm, target).await?.ok_or(ApiError::NotFound)?;
    if let Some(u) = st.store.find_by_id(target).await? {
        let payload = member_upsert_payload(realm, &u, Some(&member));
        let _ = st.emitter.emit(realm, "GUILD_MEMBER_UPDATE".into(), payload).await;
    }
    crate::routes::audit::record(
        &st, realm, actor, domain::audit::AuditAction::MemberNickUpdate, Some(target.0.raw()),
        Some(serde_json::json!({ "nick": nick }).to_string()),
    )
    .await;
    Ok(Json(member.into()))
}

/// 멤버 추방(KICK_MEMBERS) 또는 본인 탈퇴 → `GUILD_MEMBER_REMOVE` 팬아웃. 소유자는 제거 불가.
async fn remove_member<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(actor): AuthUser,
    Path((realm_id, user_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let realm = parse_realm(&realm_id)?;
    let target = resolve_user(&user_id, actor)?;

    // 소유자는 추방/탈퇴 불가 (길드 고아화 방지 — 양도 후 가능, 후속).
    if st.store.get_guild(realm).await?.map(|g| g.owner_id == target).unwrap_or(false) {
        return Err(ApiError::BadRequest("owner cannot leave or be removed".into()));
    }

    if target == actor {
        // 탈퇴: 본인이 멤버여야.
        if !st.store.is_member(realm, actor).await? {
            return Err(ApiError::NotFound);
        }
    } else {
        perm::require(&*st.store, realm, actor, Permissions::KICK_MEMBERS).await?;
    }

    if !st.store.remove_member(realm, target).await? {
        return Err(ApiError::NotFound);
    }
    let payload = member_remove_payload(realm, target);
    let _ = st.emitter.emit(realm, "GUILD_MEMBER_REMOVE".into(), payload).await;
    // 추방(타인)만 감사 기록 — 본인 탈퇴는 제외.
    if target != actor {
        crate::routes::audit::record(
            &st, realm, actor, domain::audit::AuditAction::MemberKick, Some(target.0.raw()), None,
        )
        .await;
    }
    Ok(StatusCode::NO_CONTENT)
}
