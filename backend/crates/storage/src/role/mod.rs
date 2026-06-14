//! `RoleRepository` 구현 for `PgStore` (개념: role). adapter (D17/D22).
//!
//! 권한 비트는 DB에 raw BIGINT로만 저장(계산은 domain). @everyone = id==realm_id.

use domain::id::{RealmId, Snowflake, UserId};
use domain::permissions::Permissions;
use domain::repo::{RepoError, RoleRepository};
use domain::role::{NewRole, Role};
use sqlx::Row;

use crate::store::{PgStore, map_err};

impl RoleRepository for PgStore {
    async fn create_role(&self, role: &NewRole) -> Result<(), RepoError> {
        sqlx::query("INSERT INTO roles (id, realm_id, name, permissions) VALUES ($1, $2, $3, $4)")
            .bind(role.id.0.raw() as i64)
            .bind(role.realm_id.0.raw() as i64)
            .bind(&role.name)
            .bind(role.permissions.bits() as i64)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    async fn list_roles(&self, realm_id: RealmId) -> Result<Vec<Role>, RepoError> {
        let rows = sqlx::query(
            "SELECT id, name, permissions, position FROM roles WHERE realm_id = $1 ORDER BY position, id",
        )
        .bind(realm_id.0.raw() as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(rows
            .iter()
            .map(|r| Role {
                id: domain::id::RoleId(Snowflake::from_raw(r.get::<i64, _>("id") as u64)),
                realm_id,
                name: r.get("name"),
                permissions: Permissions::from_bits_truncate(r.get::<i64, _>("permissions") as u64),
                position: r.get("position"),
            })
            .collect())
    }

    async fn assign_role(
        &self,
        realm_id: RealmId,
        user_id: UserId,
        role_id: domain::id::RoleId,
    ) -> Result<(), RepoError> {
        sqlx::query(
            "INSERT INTO member_roles (realm_id, user_id, role_id) VALUES ($1, $2, $3)
             ON CONFLICT DO NOTHING",
        )
        .bind(realm_id.0.raw() as i64)
        .bind(user_id.0.raw() as i64)
        .bind(role_id.0.raw() as i64)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(())
    }

    async fn everyone_permissions(&self, realm_id: RealmId) -> Result<Option<u64>, RepoError> {
        let row = sqlx::query("SELECT permissions FROM roles WHERE id = $1")
            .bind(realm_id.0.raw() as i64)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(row.map(|r| r.get::<i64, _>("permissions") as u64))
    }

    async fn member_role_permissions(
        &self,
        realm_id: RealmId,
        user_id: UserId,
    ) -> Result<Vec<u64>, RepoError> {
        // @everyone(id==realm_id)은 별도 경로로 처리하므로 여기선 제외.
        let rows = sqlx::query(
            "SELECT r.permissions FROM member_roles mr
             JOIN roles r ON r.id = mr.role_id
             WHERE mr.realm_id = $1 AND mr.user_id = $2 AND r.id <> $1",
        )
        .bind(realm_id.0.raw() as i64)
        .bind(user_id.0.raw() as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(rows.iter().map(|r| r.get::<i64, _>("permissions") as u64).collect())
    }

    async fn member_roles_with_ids(
        &self,
        realm_id: RealmId,
        user_id: UserId,
    ) -> Result<Vec<(u64, u64)>, RepoError> {
        let rows = sqlx::query(
            "SELECT r.id, r.permissions FROM member_roles mr
             JOIN roles r ON r.id = mr.role_id
             WHERE mr.realm_id = $1 AND mr.user_id = $2 AND r.id <> $1",
        )
        .bind(realm_id.0.raw() as i64)
        .bind(user_id.0.raw() as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(rows.iter().map(|r| (r.get::<i64, _>("id") as u64, r.get::<i64, _>("permissions") as u64)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PgStore, connect, run_migrations};
    use domain::guild::NewGuild;
    use domain::permissions::compute_guild_permissions;
    use domain::repo::GuildRepository;
    use domain::role::NewRole;

    async fn mk_user(pool: &sqlx::PgPool, id: i64, name: &str) {
        sqlx::query(
            "INSERT INTO users (id, username, email, password_hash)
             VALUES ($1, $2, $3, 'h') ON CONFLICT (id) DO NOTHING",
        )
        .bind(id).bind(name).bind(format!("{name}@e.com"))
        .execute(pool).await.unwrap();
    }

    /// 실 Postgres 필요 — skip if no DATABASE_URL. @everyone 기본 + 커스텀 역할 할당 → 유효 권한 계산.
    #[tokio::test]
    async fn role_assignment_and_effective_permissions() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL 미설정 — role 통합 테스트 skip");
            return;
        };
        let pool = connect(&url).await.expect("connect");
        run_migrations(&pool).await.expect("migrate");
        let store = PgStore::new(pool.clone());

        let (owner, mem) = (9_200_000_001i64, 9_200_000_002i64);
        let realm = RealmId(Snowflake::from_raw(9_200_000_010));
        mk_user(&pool, owner, "role_owner").await;
        mk_user(&pool, mem, "role_mem").await;
        sqlx::query("DELETE FROM realms WHERE id = $1").bind(realm.0.raw() as i64).execute(&pool).await.unwrap();

        store.create_guild(&NewGuild {
            realm_id: realm,
            name: "RoleGuild".into(),
            owner_id: UserId(Snowflake::from_raw(owner as u64)),
        }).await.unwrap();

        // @everyone 기본 권한 존재 (SEND_MESSAGES 포함, MANAGE_CHANNELS 미포함).
        let everyone = Permissions::from_bits_truncate(store.everyone_permissions(realm).await.unwrap().unwrap());
        assert!(everyone.contains(Permissions::SEND_MESSAGES));
        assert!(!everyone.contains(Permissions::MANAGE_CHANNELS));

        // 멤버 합류 + MANAGE_CHANNELS 가진 'mod' 역할 생성·할당.
        store.add_member(realm, UserId(Snowflake::from_raw(mem as u64))).await.unwrap();
        let role_id = domain::id::RoleId(Snowflake::from_raw(9_200_000_020));
        store.create_role(&NewRole {
            id: role_id,
            realm_id: realm,
            name: "mod".into(),
            permissions: Permissions::MANAGE_CHANNELS,
        }).await.unwrap();

        // 할당 전: 멤버는 @everyone만 → MANAGE_CHANNELS 없음.
        let before: Vec<Permissions> = store.member_role_permissions(realm, UserId(Snowflake::from_raw(mem as u64)))
            .await.unwrap().into_iter().map(Permissions::from_bits_truncate).collect();
        let eff_before = compute_guild_permissions(false, everyone, &before);
        assert!(!eff_before.contains(Permissions::MANAGE_CHANNELS));

        store.assign_role(realm, UserId(Snowflake::from_raw(mem as u64)), role_id).await.unwrap();

        // 할당 후: MANAGE_CHANNELS 획득.
        let after: Vec<Permissions> = store.member_role_permissions(realm, UserId(Snowflake::from_raw(mem as u64)))
            .await.unwrap().into_iter().map(Permissions::from_bits_truncate).collect();
        let eff_after = compute_guild_permissions(false, everyone, &after);
        assert!(eff_after.contains(Permissions::MANAGE_CHANNELS));
        assert!(eff_after.contains(Permissions::SEND_MESSAGES)); // @everyone 유지

        // owner는 항상 전권.
        assert_eq!(compute_guild_permissions(true, Permissions::empty(), &[]), Permissions::all());

        sqlx::query("DELETE FROM realms WHERE id = $1").bind(realm.0.raw() as i64).execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM users WHERE id = ANY($1)").bind(vec![owner, mem]).execute(&pool).await.unwrap();
    }
}
