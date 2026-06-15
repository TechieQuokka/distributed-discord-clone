//! `UserRepository` 구현 for `PgStore` (개념: user). domain port의 adapter (D22).
//!
//! username/email 유니크는 DB의 부분 유니크 인덱스(`lower(...)`, deleted_at IS NULL)가 강제 →
//! 유니크 위반은 `RepoError::Conflict`로 매핑. 조회는 소프트삭제(deleted_at) 제외.

use domain::id::{Snowflake, UserId};
use domain::repo::{RepoError, UserRepository};
use domain::user::{NewUser, User};
use sqlx::Row;

use crate::store::{PgStore, map_err};

/// SELECT 행 → User. id는 BIGINT(i64)로 저장되므로 u64로 재해석.
fn row_to_user(row: &sqlx::postgres::PgRow) -> User {
    let id: i64 = row.get("id");
    User {
        id: UserId(Snowflake::from_raw(id as u64)),
        username: row.get("username"),
        global_name: row.get("global_name"),
        email: row.get("email"),
        password_hash: row.get("password_hash"),
        is_bot: row.get("is_bot"),
    }
}

impl UserRepository for PgStore {
    async fn create_user(&self, user: &NewUser) -> Result<(), RepoError> {
        sqlx::query("INSERT INTO users (id, username, email, password_hash) VALUES ($1, $2, $3, $4)")
            .bind(user.id.0.raw() as i64)
            .bind(&user.username)
            .bind(&user.email)
            .bind(&user.password_hash)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    async fn find_by_username(&self, username: &str) -> Result<Option<User>, RepoError> {
        let row = sqlx::query(
            "SELECT id, username, global_name, email, password_hash, is_bot FROM users \
             WHERE lower(username) = lower($1) AND deleted_at IS NULL",
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(row.as_ref().map(row_to_user))
    }

    async fn find_by_id(&self, id: UserId) -> Result<Option<User>, RepoError> {
        let row = sqlx::query(
            "SELECT id, username, global_name, email, password_hash, is_bot FROM users \
             WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(id.0.raw() as i64)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(row.as_ref().map(row_to_user))
    }

    async fn set_totp_secret(&self, id: UserId, secret: Option<&[u8]>) -> Result<(), RepoError> {
        sqlx::query("UPDATE users SET mfa_totp_secret = $2 WHERE id = $1")
            .bind(id.0.raw() as i64)
            .bind(secret)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    async fn totp_secret(&self, id: UserId) -> Result<Option<Vec<u8>>, RepoError> {
        let row =
            sqlx::query("SELECT mfa_totp_secret FROM users WHERE id = $1 AND deleted_at IS NULL")
                .bind(id.0.raw() as i64)
                .fetch_optional(&self.pool)
                .await
                .map_err(map_err)?;
        Ok(row.and_then(|r| r.get::<Option<Vec<u8>>, _>("mfa_totp_secret")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 실제 Postgres 필요 — `DATABASE_URL` 미설정 시 skip.
    #[tokio::test]
    async fn create_find_and_conflict() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL 미설정 — user 테스트 skip");
            return;
        };
        let pool = crate::connect(&url).await.expect("connect");
        crate::run_migrations(&pool).await.expect("migrate");
        let store = PgStore::new(pool.clone());

        let id = UserId(Snowflake::from_raw(987_654_321));
        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(id.0.raw() as i64)
            .execute(&pool)
            .await
            .unwrap();

        let nu = NewUser {
            id,
            username: "Bob".into(),
            email: "bob@example.com".into(),
            password_hash: "$argon2id$dummy".into(),
        };
        store.create_user(&nu).await.expect("create");

        let found = store.find_by_username("bob").await.unwrap().expect("found");
        assert_eq!(found.id, id);
        assert_eq!(found.email, "bob@example.com");
        assert_eq!(store.find_by_id(id).await.unwrap().unwrap().username, "Bob");

        let dup = NewUser {
            id: UserId(Snowflake::from_raw(111)),
            username: "BOB".into(),
            email: "other@example.com".into(),
            password_hash: "x".into(),
        };
        assert!(matches!(store.create_user(&dup).await, Err(RepoError::Conflict)));

        assert!(store.find_by_username("nobody_xyz").await.unwrap().is_none());

        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(id.0.raw() as i64)
            .execute(&pool)
            .await
            .unwrap();
    }
}
