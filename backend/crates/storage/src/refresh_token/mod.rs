//! `RefreshTokenRepository` 구현 for `PgStore` (개념: refresh_token). adapter (D14/D22).
//!
//! 활성 토큰 = `revoked_at IS NULL AND expires_at > now`. 해시는 유니크.
//! 만료는 도메인에선 unix seconds, 컬럼은 TIMESTAMPTZ → `to_timestamp()`로 변환.

use domain::id::{RefreshTokenId, Snowflake, UserId};
use domain::refresh_token::{NewRefreshToken, RefreshToken};
use domain::repo::{RefreshTokenRepository, RepoError};
use sqlx::Row;

use crate::store::{PgStore, map_err};

fn row_to_token(r: &sqlx::postgres::PgRow) -> RefreshToken {
    let id: i64 = r.get("id");
    let user_id: i64 = r.get("user_id");
    RefreshToken {
        id: RefreshTokenId(Snowflake::from_raw(id as u64)),
        user_id: UserId(Snowflake::from_raw(user_id as u64)),
    }
}

impl RefreshTokenRepository for PgStore {
    async fn create_refresh_token(&self, t: &NewRefreshToken) -> Result<(), RepoError> {
        sqlx::query(
            "INSERT INTO refresh_tokens (id, user_id, token_hash, rotated_from, expires_at) \
             VALUES ($1, $2, $3, $4, to_timestamp($5))",
        )
        .bind(t.id.0.raw() as i64)
        .bind(t.user_id.0.raw() as i64)
        .bind(&t.token_hash)
        .bind(t.rotated_from.map(|r| r.0.raw() as i64))
        .bind(t.expires_at_unix)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(())
    }

    async fn find_active(
        &self,
        token_hash: &[u8],
        now_unix: i64,
    ) -> Result<Option<RefreshToken>, RepoError> {
        let row = sqlx::query(
            "SELECT id, user_id FROM refresh_tokens \
             WHERE token_hash = $1 AND revoked_at IS NULL AND expires_at > to_timestamp($2)",
        )
        .bind(token_hash)
        .bind(now_unix)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(row.as_ref().map(row_to_token))
    }

    async fn find_by_hash(&self, token_hash: &[u8]) -> Result<Option<RefreshToken>, RepoError> {
        let row = sqlx::query("SELECT id, user_id FROM refresh_tokens WHERE token_hash = $1")
            .bind(token_hash)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(row.as_ref().map(row_to_token))
    }

    async fn revoke(&self, id: RefreshTokenId, now_unix: i64) -> Result<(), RepoError> {
        sqlx::query(
            "UPDATE refresh_tokens SET revoked_at = to_timestamp($2) \
             WHERE id = $1 AND revoked_at IS NULL",
        )
        .bind(id.0.raw() as i64)
        .bind(now_unix)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(())
    }

    async fn revoke_all_for_user(&self, user_id: UserId, now_unix: i64) -> Result<(), RepoError> {
        sqlx::query(
            "UPDATE refresh_tokens SET revoked_at = to_timestamp($2) \
             WHERE user_id = $1 AND revoked_at IS NULL",
        )
        .bind(user_id.0.raw() as i64)
        .bind(now_unix)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::repo::UserRepository;
    use domain::user::NewUser;

    /// 실제 Postgres 필요 — `DATABASE_URL` 미설정 시 skip.
    /// create → find_active → revoke → 재사용 탐지(폐기 토큰 재제시) 종단 검증.
    #[tokio::test]
    async fn rotation_and_reuse_detection() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL 미설정 — PgRefreshTokenRepository 테스트 skip");
            return;
        };
        let pool = crate::connect(&url).await.expect("connect");
        crate::run_migrations(&pool).await.expect("migrate");

        let store = PgStore::new(pool.clone());
        let users = &store;
        let repo = &store;

        let uid = UserId(Snowflake::from_raw(515_151));
        sqlx::query("DELETE FROM users WHERE id = $1").bind(uid.0.raw() as i64).execute(&pool).await.unwrap();
        users
            .create_user(&NewUser {
                id: uid,
                username: "rt_user".into(),
                email: "rt@example.com".into(),
                password_hash: "x".into(),
            })
            .await
            .unwrap();

        let t1 = RefreshTokenId(Snowflake::from_raw(515_152));
        let t2 = RefreshTokenId(Snowflake::from_raw(515_153));
        let h1 = vec![9u8, 9, 9];
        let h2 = vec![8u8, 8, 8];
        let now = 1_700_000_000;

        repo.create_refresh_token(&NewRefreshToken { id: t1, user_id: uid, token_hash: h1.clone(), rotated_from: None, expires_at_unix: now + 1000 }).await.unwrap();

        // 활성 조회 + 만료 경계.
        assert_eq!(repo.find_active(&h1, now).await.unwrap().unwrap().id, t1);
        assert!(repo.find_active(&h1, now + 2000).await.unwrap().is_none());

        // 회전: t1 폐기 → t2 발급(rotated_from = t1).
        repo.revoke(t1, now).await.unwrap();
        repo.create_refresh_token(&NewRefreshToken { id: t2, user_id: uid, token_hash: h2.clone(), rotated_from: Some(t1), expires_at_unix: now + 1000 }).await.unwrap();
        assert!(repo.find_active(&h1, now).await.unwrap().is_none(), "폐기된 t1은 비활성");
        assert_eq!(repo.find_active(&h2, now).await.unwrap().unwrap().id, t2);

        // 재사용 탐지: 폐기된 t1 재제시 → find_by_hash로 user 식별 → 전체 폐기.
        let seen = repo.find_by_hash(&h1).await.unwrap().expect("seen");
        assert_eq!(seen.user_id, uid);
        repo.revoke_all_for_user(uid, now).await.unwrap();
        assert!(repo.find_active(&h2, now).await.unwrap().is_none(), "체인 전체 무효화");

        sqlx::query("DELETE FROM users WHERE id = $1").bind(uid.0.raw() as i64).execute(&pool).await.unwrap();
    }
}
