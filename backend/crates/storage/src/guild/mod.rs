//! `GuildRepository` 구현 for `PgStore` (개념: guild). adapter (DB-D1/D22).
//!
//! 길드 생성 = realms + guilds + 소유자 members 한 트랜잭션(원자성).

use domain::guild::{Guild, NewGuild};
use domain::id::{RealmId, Snowflake, UserId};
use domain::permissions::Permissions;
use domain::repo::{GuildRepository, RepoError};
use sqlx::Row;

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
