//! 역할 라우트 (개념: routes/role). 역할 생성/목록 + 멤버 역할 부여 (D17).
//!
//! 생성·부여는 `MANAGE_ROLES` 필요. 권한 비트는 raw u64로 주고받고 계산은 domain.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, put};
use domain::id::{RealmId, RoleId, Snowflake, UserId};
use domain::permissions::Permissions;
use domain::repo::Store;
use domain::role::NewRole;
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::extract::AuthUser;
use crate::perm;
use crate::state::AppState;

pub fn routes<S: Store + 'static>() -> axum::Router<AppState<S>> {
    axum::Router::new()
        .route("/guilds/{realm_id}/roles", get(list_roles::<S>).post(create_role::<S>))
        .route("/guilds/{realm_id}/members/{user_id}/roles/{role_id}", put(assign_role::<S>))
}

fn parse_realm(s: &str) -> Result<RealmId, ApiError> {
    s.parse::<u64>().map(|n| RealmId(Snowflake::from_raw(n))).map_err(|_| ApiError::BadRequest("invalid guild id".into()))
}
fn parse_u64(s: &str, what: &'static str) -> Result<u64, ApiError> {
    s.parse::<u64>().map_err(|_| ApiError::BadRequest(format!("invalid {what}")))
}

#[derive(Deserialize)]
pub struct CreateRoleReq {
    pub name: String,
    /// 권한 비트마스크 (raw u64). 미지정 시 0(권한 없음).
    #[serde(default)]
    pub permissions: u64,
}

#[derive(Serialize)]
pub struct RoleView {
    pub id: String,
    pub name: String,
    pub permissions: String, // u64는 JSON에서 문자열(정밀도 안전, id 규약과 일관)
    pub position: i32,
}

async fn list_roles<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(user): AuthUser,
    Path(realm_id): Path<String>,
) -> Result<Json<Vec<RoleView>>, ApiError> {
    let realm = parse_realm(&realm_id)?;
    if !st.store.is_member(realm, user).await? {
        return Err(ApiError::Forbidden);
    }
    let roles = st.store.list_roles(realm).await?;
    Ok(Json(
        roles
            .into_iter()
            .map(|r| RoleView {
                id: r.id.0.raw().to_string(),
                name: r.name,
                permissions: r.permissions.bits().to_string(),
                position: r.position,
            })
            .collect(),
    ))
}

async fn create_role<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(user): AuthUser,
    Path(realm_id): Path<String>,
    Json(req): Json<CreateRoleReq>,
) -> Result<(StatusCode, Json<RoleView>), ApiError> {
    let realm = parse_realm(&realm_id)?;
    perm::require(&*st.store, realm, user, Permissions::MANAGE_ROLES).await?;

    let name = req.name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest("role name is required".into()));
    }
    // 부여하려는 권한이 자기 유효권한을 넘지 못하게(권한 상승 방지). owner/admin은 통과.
    let mine = perm::effective(&*st.store, realm, user).await?;
    let requested = Permissions::from_bits_truncate(req.permissions);
    if !mine.contains(Permissions::ADMINISTRATOR) && !mine.contains(requested) {
        return Err(ApiError::Forbidden);
    }

    let id = RoleId(st.snowflakes.next(st.clock.now_ms()));
    st.store
        .create_role(&NewRole { id, realm_id: realm, name: name.to_owned(), permissions: requested })
        .await?;
    crate::routes::audit::record(
        &st, realm, user, domain::audit::AuditAction::RoleCreate, Some(id.0.raw()),
        Some(serde_json::json!({ "name": name, "permissions": requested.bits().to_string() }).to_string()),
    )
    .await;
    Ok((
        StatusCode::CREATED,
        Json(RoleView {
            id: id.0.raw().to_string(),
            name: name.to_owned(),
            permissions: requested.bits().to_string(),
            position: 0,
        }),
    ))
}

async fn assign_role<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(actor): AuthUser,
    Path((realm_id, user_id, role_id)): Path<(String, String, String)>,
) -> Result<StatusCode, ApiError> {
    let realm = parse_realm(&realm_id)?;
    let target = UserId(Snowflake::from_raw(parse_u64(&user_id, "user id")?));
    let role = RoleId(Snowflake::from_raw(parse_u64(&role_id, "role id")?));
    perm::require(&*st.store, realm, actor, Permissions::MANAGE_ROLES).await?;

    if !st.store.is_member(realm, target).await? {
        return Err(ApiError::BadRequest("target user is not a member".into()));
    }
    st.store.assign_role(realm, target, role).await?;
    crate::routes::audit::record(
        &st, realm, actor, domain::audit::AuditAction::MemberRoleUpdate, Some(target.0.raw()),
        Some(serde_json::json!({ "role_id": role.0.raw().to_string() }).to_string()),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}
