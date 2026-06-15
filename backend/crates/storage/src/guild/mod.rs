//! `GuildRepository` 구현 for `PgStore` (개념: guild). adapter (DB-D1/D22).
//!
//! 길드 생성 = realms + guilds + 소유자 members 한 트랜잭션(원자성).

use domain::guild::{Guild, NewGuild};
use domain::id::{RealmId, RoleId, Snowflake, UserId};
use domain::member::Member;
use domain::permissions::Permissions;
use domain::repo::{GuildRepository, RepoError};
use sqlx::Row;

/// 한 행(members + array_agg(member_roles))을 `Member`로.
fn row_to_member(realm_id: RealmId, r: &sqlx::postgres::PgRow) -> Member {
    let roles: Vec<i64> = r.get("roles");
    Member {
        realm_id,
        user_id: UserId(Snowflake::from_raw(r.get::<i64, _>("user_id") as u64)),
        nick: r.get("nick"),
        joined_at: r.get("joined"),
        roles: roles.into_iter().map(|x| RoleId(Snowflake::from_raw(x as u64))).collect(),
    }
}

use crate::store::{PgStore, map_err};

impl GuildRepository for PgStore {
    async fn create_guild(&self, g: &NewGuild) -> Result<(), RepoError> {
        let mut tx = self.pool.begin().await.map_err(map_err)?;

        sqlx::query("INSERT INTO realms (id, kind, name) VALUES ($1, 'guild', $2)")
            .bind(g.realm_id.0.raw() as i64)
            .bind(&g.name)
            .execute(&mut *tx)
            .await
            .map_err(map_err)?;

        sqlx::query("INSERT INTO guilds (realm_id, name, owner_id) VALUES ($1, $2, $3)")
            .bind(g.realm_id.0.raw() as i64)
            .bind(&g.name)
            .bind(g.owner_id.0.raw() as i64)
            .execute(&mut *tx)
            .await
            .map_err(map_err)?;

        // @everyone 역할 (id == realm_id 규약, D17). 모든 멤버가 암묵 보유.
        sqlx::query("INSERT INTO roles (id, realm_id, name, permissions) VALUES ($1, $1, '@everyone', $2)")
            .bind(g.realm_id.0.raw() as i64)
            .bind(Permissions::default_everyone().bits() as i64)
            .execute(&mut *tx)
            .await
            .map_err(map_err)?;

        sqlx::query("INSERT INTO members (realm_id, user_id) VALUES ($1, $2)")
            .bind(g.realm_id.0.raw() as i64)
            .bind(g.owner_id.0.raw() as i64)
            .execute(&mut *tx)
            .await
            .map_err(map_err)?;

        tx.commit().await.map_err(map_err)?;
        Ok(())
    }

    async fn get_guild(&self, realm_id: RealmId) -> Result<Option<Guild>, RepoError> {
        let row = sqlx::query("SELECT name, owner_id FROM guilds WHERE realm_id = $1")
            .bind(realm_id.0.raw() as i64)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(row.map(|r| Guild {
            realm_id,
            name: r.get("name"),
            owner_id: UserId(Snowflake::from_raw(r.get::<i64, _>("owner_id") as u64)),
        }))
    }

    async fn add_member(&self, realm_id: RealmId, user_id: UserId) -> Result<(), RepoError> {
        sqlx::query(
            "INSERT INTO members (realm_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        )
        .bind(realm_id.0.raw() as i64)
        .bind(user_id.0.raw() as i64)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(())
    }

    async fn is_member(&self, realm_id: RealmId, user_id: UserId) -> Result<bool, RepoError> {
        let row =
            sqlx::query("SELECT 1 AS x FROM members WHERE realm_id = $1 AND user_id = $2")
                .bind(realm_id.0.raw() as i64)
                .bind(user_id.0.raw() as i64)
                .fetch_optional(&self.pool)
                .await
                .map_err(map_err)?;
        Ok(row.is_some())
    }

    async fn get_member(
        &self,
        realm_id: RealmId,
        user_id: UserId,
    ) -> Result<Option<Member>, RepoError> {
        let row = sqlx::query(
            "SELECT m.user_id, m.nick, EXTRACT(EPOCH FROM m.joined_at)::bigint AS joined,
                    COALESCE(array_agg(mr.role_id) FILTER (WHERE mr.role_id IS NOT NULL), '{}') AS roles
             FROM members m
             LEFT JOIN member_roles mr ON mr.realm_id = m.realm_id AND mr.user_id = m.user_id
             WHERE m.realm_id = $1 AND m.user_id = $2
             GROUP BY m.user_id, m.nick, m.joined_at",
        )
        .bind(realm_id.0.raw() as i64)
        .bind(user_id.0.raw() as i64)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(row.map(|r| row_to_member(realm_id, &r)))
    }

    async fn list_members(&self, realm_id: RealmId) -> Result<Vec<Member>, RepoError> {
        let rows = sqlx::query(
            "SELECT m.user_id, m.nick, EXTRACT(EPOCH FROM m.joined_at)::bigint AS joined,
                    COALESCE(array_agg(mr.role_id) FILTER (WHERE mr.role_id IS NOT NULL), '{}') AS roles
             FROM members m
             LEFT JOIN member_roles mr ON mr.realm_id = m.realm_id AND mr.user_id = m.user_id
             WHERE m.realm_id = $1
             GROUP BY m.user_id, m.nick, m.joined_at
             ORDER BY m.joined_at ASC",
        )
        .bind(realm_id.0.raw() as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(rows.iter().map(|r| row_to_member(realm_id, r)).collect())
    }

    async fn update_member_nick(
        &self,
        realm_id: RealmId,
        user_id: UserId,
        nick: Option<&str>,
    ) -> Result<bool, RepoError> {
        let res = sqlx::query("UPDATE members SET nick = $3 WHERE realm_id = $1 AND user_id = $2")
            .bind(realm_id.0.raw() as i64)
            .bind(user_id.0.raw() as i64)
            .bind(nick)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(res.rows_affected() > 0)
    }

    async fn remove_member(&self, realm_id: RealmId, user_id: UserId) -> Result<bool, RepoError> {
        let res = sqlx::query("DELETE FROM members WHERE realm_id = $1 AND user_id = $2")
            .bind(realm_id.0.raw() as i64)
            .bind(user_id.0.raw() as i64)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(res.rows_affected() > 0)
    }

    async fn member_realm_ids(&self, user_id: UserId) -> Result<Vec<RealmId>, RepoError> {
        let rows = sqlx::query("SELECT realm_id FROM members WHERE user_id = $1")
            .bind(user_id.0.raw() as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(rows
            .iter()
            .map(|r| {
                let id: i64 = r.get("realm_id");
                RealmId(Snowflake::from_raw(id as u64))
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PgStore, connect, run_migrations};
    use domain::guild::NewGuild;
    use domain::repo::RoleRepository;
    use domain::role::NewRole;

    async fn mk_user(pool: &sqlx::PgPool, id: i64, name: &str) {
        sqlx::query(
            "INSERT INTO users (id, username, email, password_hash)
             VALUES ($1, $2, $3, 'h') ON CONFLICT (id) DO NOTHING",
        )
        .bind(id)
        .bind(name)
        .bind(format!("{name}@e.com"))
        .execute(pool)
        .await
        .unwrap();
    }

    /// 실제 Postgres 필요 — `DATABASE_URL` 미설정 시 skip. 멤버 조회/목록/닉수정/제거 + 역할 array_agg.
    #[tokio::test]
    async fn member_lifecycle_list_nick_roles_remove() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL 미설정 — member 통합 테스트 skip");
            return;
        };
        let pool = connect(&url).await.expect("connect");
        run_migrations(&pool).await.expect("migrate");
        let store = PgStore::new(pool.clone());

        let (owner, joiner) = (9_300_000_001i64, 9_300_000_002i64);
        let realm = RealmId(Snowflake::from_raw(9_300_000_010));
        let role_id = RoleId(Snowflake::from_raw(9_300_000_020));
        mk_user(&pool, owner, "mem_owner").await;
        mk_user(&pool, joiner, "mem_joiner").await;
        sqlx::query("DELETE FROM realms WHERE id = $1").bind(realm.0.raw() as i64).execute(&pool).await.unwrap();

        store
            .create_guild(&NewGuild {
                realm_id: realm,
                name: "MemGuild".into(),
                owner_id: UserId(Snowflake::from_raw(owner as u64)),
            })
            .await
            .unwrap();
        let joiner_uid = UserId(Snowflake::from_raw(joiner as u64));
        store.add_member(realm, joiner_uid).await.unwrap();

        // 역할 생성 + joiner에 부여 → array_agg 검증.
        store
            .create_role(&NewRole {
                id: role_id,
                realm_id: realm,
                name: "vip".into(),
                permissions: Permissions::empty(),
            })
            .await
            .unwrap();
        store.assign_role(realm, joiner_uid, role_id).await.unwrap();

        // 목록 = owner + joiner.
        let list = store.list_members(realm).await.unwrap();
        assert_eq!(list.len(), 2);

        // 단건: joiner — nick None, roles=[role_id].
        let m = store.get_member(realm, joiner_uid).await.unwrap().unwrap();
        assert_eq!(m.nick, None);
        assert_eq!(m.roles, vec![role_id]);

        // nick 설정 → 반영.
        assert!(store.update_member_nick(realm, joiner_uid, Some("Nicky")).await.unwrap());
        assert_eq!(store.get_member(realm, joiner_uid).await.unwrap().unwrap().nick, Some("Nicky".into()));
        // nick 제거.
        assert!(store.update_member_nick(realm, joiner_uid, None).await.unwrap());
        assert_eq!(store.get_member(realm, joiner_uid).await.unwrap().unwrap().nick, None);
        // 비멤버 nick 수정 → false.
        let ghost = UserId(Snowflake::from_raw(9_300_000_099));
        assert!(!store.update_member_nick(realm, ghost, Some("x")).await.unwrap());

        // 제거 → member_roles CASCADE 정리, is_member false.
        assert!(store.remove_member(realm, joiner_uid).await.unwrap());
        assert!(!store.is_member(realm, joiner_uid).await.unwrap());
        assert!(store.get_member(realm, joiner_uid).await.unwrap().is_none());
        assert!(!store.remove_member(realm, joiner_uid).await.unwrap()); // 멱등(이미 없음).

        // 정리.
        sqlx::query("DELETE FROM realms WHERE id = $1").bind(realm.0.raw() as i64).execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM users WHERE id = ANY($1)").bind(vec![owner, joiner]).execute(&pool).await.unwrap();
    }
}
