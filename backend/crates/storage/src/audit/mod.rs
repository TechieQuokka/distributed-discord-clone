//! `AuditRepository` 구현 for `PgStore` (개념: audit). adapter (D22).
//! `changes`는 jsonb지만 text로 주고받아 storage가 serde 무의존(생산 엣지가 JSON 문자열 조립).

use domain::audit::{AuditAction, AuditEntry, NewAuditEntry};
use domain::id::{RealmId, Snowflake, UserId};
use domain::repo::{AuditRepository, RepoError};
use sqlx::Row;

use crate::store::{PgStore, map_err};

fn row_to_entry(r: &sqlx::postgres::PgRow) -> AuditEntry {
    let id: i64 = r.get("id");
    let realm_id: i64 = r.get("realm_id");
    let actor: Option<i64> = r.get("actor_id");
    let action_type: i16 = r.get("action_type");
    let target: Option<i64> = r.get("target_id");
    AuditEntry {
        id: Snowflake::from_raw(id as u64),
        realm_id: RealmId(Snowflake::from_raw(realm_id as u64)),
        actor_id: actor.map(|a| UserId(Snowflake::from_raw(a as u64))),
        action: AuditAction::from_code(action_type).unwrap_or(AuditAction::ChannelCreate),
        target_id: target.map(|t| t as u64),
        changes: r.get("changes"),
    }
}

impl AuditRepository for PgStore {
    async fn log_audit(&self, e: &NewAuditEntry) -> Result<(), RepoError> {
        sqlx::query(
            "INSERT INTO audit_log_entries (id, realm_id, actor_id, action_type, target_id, changes) \
             VALUES ($1, $2, $3, $4, $5, $6::jsonb)",
        )
        .bind(e.id.raw() as i64)
        .bind(e.realm_id.0.raw() as i64)
        .bind(e.actor_id.0.raw() as i64)
        .bind(e.action.code())
        .bind(e.target_id.map(|t| t as i64))
        .bind(&e.changes)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(())
    }

    async fn list_audit(
        &self,
        realm: RealmId,
        before: Option<u64>,
        limit: i64,
    ) -> Result<Vec<AuditEntry>, RepoError> {
        let rows = sqlx::query(
            "SELECT id, realm_id, actor_id, action_type, target_id, changes::text AS changes \
             FROM audit_log_entries \
             WHERE realm_id = $1 AND ($2::bigint IS NULL OR id < $2) \
             ORDER BY id DESC LIMIT $3",
        )
        .bind(realm.0.raw() as i64)
        .bind(before.map(|b| b as i64))
        .bind(limit.clamp(1, 100))
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(rows.iter().map(row_to_entry).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PgStore, connect, run_migrations};
    use domain::guild::NewGuild;
    use domain::repo::{GuildRepository, UserRepository};
    use domain::user::NewUser;

    /// 실제 Postgres 필요 — skip if no DATABASE_URL. 감사 기록/목록(최신순)/커서.
    #[tokio::test]
    async fn audit_log_and_list() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL 미설정 — audit 테스트 skip");
            return;
        };
        let pool = connect(&url).await.expect("connect");
        run_migrations(&pool).await.expect("migrate");
        let s = PgStore::new(pool.clone());

        let actor = UserId(Snowflake::from_raw(770_001));
        let realm = RealmId(Snowflake::from_raw(770_002));
        sqlx::query("DELETE FROM realms WHERE id = $1").bind(realm.0.raw() as i64).execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM users WHERE id = $1").bind(actor.0.raw() as i64).execute(&pool).await.unwrap();
        s.create_user(&NewUser { id: actor, username: "au_actor".into(), email: "au@e.com".into(), password_hash: "x".into() }).await.unwrap();
        s.create_guild(&NewGuild { realm_id: realm, name: "G".into(), owner_id: actor }).await.unwrap();

        let e1 = Snowflake::from_raw(770_010);
        let e2 = Snowflake::from_raw(770_011);
        s.log_audit(&NewAuditEntry { id: e1, realm_id: realm, actor_id: actor, action: AuditAction::RoleCreate, target_id: Some(555), changes: Some(r#"{"name":"mod"}"#.into()) }).await.unwrap();
        s.log_audit(&NewAuditEntry { id: e2, realm_id: realm, actor_id: actor, action: AuditAction::MemberKick, target_id: Some(999), changes: None }).await.unwrap();

        let list = s.list_audit(realm, None, 50).await.unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, e2, "최신순");
        assert_eq!(list[0].action, AuditAction::MemberKick);
        assert_eq!(list[0].target_id, Some(999));
        assert_eq!(list[1].action, AuditAction::RoleCreate);
        assert!(list[1].changes.as_deref().unwrap().contains("mod"));

        // 커서: e2 이전 → e1만.
        let older = s.list_audit(realm, Some(e2.raw()), 50).await.unwrap();
        assert_eq!(older.len(), 1);
        assert_eq!(older[0].id, e1);

        sqlx::query("DELETE FROM realms WHERE id = $1").bind(realm.0.raw() as i64).execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM users WHERE id = $1").bind(actor.0.raw() as i64).execute(&pool).await.unwrap();
    }
}
