//! 감사 로그 라우트 (개념: routes/audit). `GET /guilds/:id/audit-logs` (VIEW_AUDIT_LOG).
//!
//! 기록(log)은 각 mutation 라우트가 `Store::log_audit`로 직접 남긴다(best-effort). 여기선 조회만.

use axum::Json;
use axum::extract::{Path, Query, State};
use domain::id::{RealmId, Snowflake};
use domain::permissions::Permissions;
use domain::repo::Store;
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::extract::AuthUser;
use crate::state::AppState;

/// 감사 항목 기록 헬퍼 (best-effort — 실패해도 주 동작 진행). 각 mutation 라우트가 호출.
pub async fn record<S: Store>(
    st: &AppState<S>,
    realm: RealmId,
    actor: domain::id::UserId,
    action: domain::audit::AuditAction,
    target: Option<u64>,
    changes: Option<String>,
) {
    let id = st.snowflakes.next(st.clock.now_ms());
    let entry = domain::audit::NewAuditEntry { id, realm_id: realm, actor_id: actor, action, target_id: target, changes };
    if let Err(e) = st.store.log_audit(&entry).await {
        tracing::warn!(error = %e, "audit log failed");
    }
}

pub fn routes<S: Store + 'static>() -> axum::Router<AppState<S>> {
    axum::Router::new().route("/guilds/{realm_id}/audit-logs", axum::routing::get(list_audit::<S>))
}

#[derive(Deserialize)]
pub struct AuditQuery {
    pub before: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Serialize)]
pub struct AuditEntryView {
    pub id: String,
    pub actor_id: Option<String>,
    pub action_type: i16,
    pub target_id: Option<String>,
    pub changes: Option<serde_json::Value>,
}

async fn list_audit<S: Store + 'static>(
    State(st): State<AppState<S>>,
    AuthUser(user): AuthUser,
    Path(realm_id): Path<String>,
    Query(q): Query<AuditQuery>,
) -> Result<Json<Vec<AuditEntryView>>, ApiError> {
    let realm = realm_id
        .parse::<u64>()
        .map(|n| RealmId(Snowflake::from_raw(n)))
        .map_err(|_| ApiError::BadRequest("invalid guild id".into()))?;
    crate::perm::require(&*st.store, realm, user, Permissions::VIEW_AUDIT_LOG).await?;

    let before = match q.before {
        Some(s) => Some(s.parse::<u64>().map_err(|_| ApiError::BadRequest("invalid before cursor".into()))?),
        None => None,
    };
    let entries = st.store.list_audit(realm, before, q.limit.unwrap_or(50)).await?;
    Ok(Json(
        entries
            .into_iter()
            .map(|e| AuditEntryView {
                id: e.id.raw().to_string(),
                actor_id: e.actor_id.map(|a| a.0.raw().to_string()),
                action_type: e.action.code(),
                target_id: e.target_id.map(|t| t.to_string()),
                // changes(JSON 문자열)를 다시 Value로 — 클라엔 객체로 노출.
                changes: e.changes.and_then(|c| serde_json::from_str(&c).ok()),
            })
            .collect(),
    ))
}
