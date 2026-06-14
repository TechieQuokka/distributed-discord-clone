//! `InviteRepository` 구현 for `PgStore` (개념: invite). adapter (D22/D28).
//!
//! redeem은 **한 트랜잭션**: 행을 `FOR UPDATE`로 잠그고 유효성(만료/소진) 검사 →
//! 멤버 삽입(멱등) → uses 증가. 동시 redeem 경합에도 정확한 uses 카운트.

use domain::id::{ChannelId, RealmId, Snowflake, UserId};
use domain::invite::{Invite, NewInvite};
use domain::repo::{InviteRepository, RepoError};
use sqlx::Row;

use crate::store::{PgStore, map_err};

fn opt_id<T>(v: Option<i64>, wrap: impl Fn(Snowflake) -> T) -> Option<T> {
    v.map(|n| wrap(Snowflake::from_raw(n as u64)))
}

impl InviteRepository for PgStore {
    async fn create_invite(&self, inv: &NewInvite) -> Result<(), RepoError> {
        sqlx::query(
            "INSERT INTO invites (code, realm_id, channel_id, inviter_id, max_uses, expires_at)
             VALUES ($1, $2, $3, $4, $5, to_timestamp($6))",
        )
        .bind(&inv.code)
        .bind(inv.realm_id.0.raw() as i64)
        .bind(inv.channel_id.map(|c| c.0.raw() as i64))
        .bind(inv.inviter_id.map(|u| u.0.raw() as i64))
        .bind(inv.max_uses)
        .bind(inv.expires_at) // None → to_timestamp(NULL) = NULL (무기한)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(())
    }

    async fn find_invite(&self, code: &str) -> Result<Option<Invite>, RepoError> {
        let row = sqlx::query(
            "SELECT code, realm_id, channel_id, inviter_id, uses, max_uses,
                    EXTRACT(EPOCH FROM expires_at)::bigint AS expires_unix
             FROM invites WHERE code = $1",
        )
        .bind(code)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(row.map(|r| Invite {
            code: r.get("code"),
            realm_id: RealmId(Snowflake::from_raw(r.get::<i64, _>("realm_id") as u64)),
            channel_id: opt_id(r.get("channel_id"), ChannelId),
            inviter_id: opt_id(r.get("inviter_id"), UserId),
            uses: r.get("uses"),
            max_uses: r.get("max_uses"),
            expires_at: r.get("expires_unix"),
        }))
    }

    async fn redeem_invite(
        &self,
        code: &str,
        user: UserId,
        now_unix: i64,
    ) -> Result<Option<RealmId>, RepoError> {
        let mut tx = self.pool.begin().await.map_err(map_err)?;

        let row = sqlx::query(
            "SELECT realm_id, uses, max_uses,
                    EXTRACT(EPOCH FROM expires_at)::bigint AS expires_unix
             FROM invites WHERE code = $1 FOR UPDATE",
        )
        .bind(code)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_err)?;

        let Some(row) = row else {
            return Ok(None); // 미존재.
        };
        let realm_raw: i64 = row.get("realm_id");
        let uses: i32 = row.get("uses");
        let max_uses: i32 = row.get("max_uses");
        let expires: Option<i64> = row.get("expires_unix");

        let expired = expires.map(|e| now_unix >= e).unwrap_or(false);
        let exhausted = max_uses > 0 && uses >= max_uses;
        if expired || exhausted {
            return Ok(None); // 무효 — 트랜잭션 롤백(드롭).
        }

        sqlx::query("INSERT INTO members (realm_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
            .bind(realm_raw)
            .bind(user.0.raw() as i64)
            .execute(&mut *tx)
            .await
            .map_err(map_err)?;

        sqlx::query("UPDATE invites SET uses = uses + 1 WHERE code = $1")
            .bind(code)
            .execute(&mut *tx)
            .await
            .map_err(map_err)?;

        tx.commit().await.map_err(map_err)?;
        Ok(Some(RealmId(Snowflake::from_raw(realm_raw as u64))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PgStore, connect, run_migrations};
    use domain::guild::NewGuild;
    use domain::repo::GuildRepository;

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

    /// 실제 Postgres 필요 — `DATABASE_URL` 미설정 시 skip. create→redeem→멱등·소진·만료·미존재.
    #[tokio::test]
    async fn invite_create_and_redeem_lifecycle() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL 미설정 — invite 통합 테스트 skip");
            return;
        };
        let pool = connect(&url).await.expect("connect");
        run_migrations(&pool).await.expect("migrate");
        let store = PgStore::new(pool.clone());

        // 고유 id (테스트 격리). owner + 합류자 2명.
        let (owner, joiner) = (9_100_000_001i64, 9_100_000_002i64);
        let realm = RealmId(Snowflake::from_raw(9_100_000_010));
        mk_user(&pool, owner, "inv_owner").await;
        mk_user(&pool, joiner, "inv_joiner").await;
        // 깨끗한 시작.
        sqlx::query("DELETE FROM realms WHERE id = $1").bind(realm.0.raw() as i64).execute(&pool).await.unwrap();

        store
            .create_guild(&NewGuild {
                realm_id: realm,
                name: "InvGuild".into(),
                owner_id: UserId(Snowflake::from_raw(owner as u64)),
            })
            .await
            .unwrap();

        // max_uses=1 초대 생성.
        store
            .create_invite(&NewInvite {
                code: "TESTCODE".into(),
                realm_id: realm,
                channel_id: None,
                inviter_id: Some(UserId(Snowflake::from_raw(owner as u64))),
                max_uses: 1,
                expires_at: None,
            })
            .await
            .unwrap();

        let found = store.find_invite("TESTCODE").await.unwrap().unwrap();
        assert_eq!(found.realm_id, realm);
        assert_eq!(found.uses, 0);

        // redeem → 합류자 멤버 + uses=1.
        let joined = store
            .redeem_invite("TESTCODE", UserId(Snowflake::from_raw(joiner as u64)), 1_000)
            .await
            .unwrap();
        assert_eq!(joined, Some(realm));
        assert!(store.is_member(realm, UserId(Snowflake::from_raw(joiner as u64))).await.unwrap());
        assert_eq!(store.find_invite("TESTCODE").await.unwrap().unwrap().uses, 1);

        // max_uses=1 소진 → 두 번째 redeem 무효.
        let again = store
            .redeem_invite("TESTCODE", UserId(Snowflake::from_raw(owner as u64)), 1_000)
            .await
            .unwrap();
        assert_eq!(again, None, "소진된 초대는 redeem 불가");

        // 미존재 코드 → None.
        assert_eq!(store.redeem_invite("NOPE", UserId(Snowflake::from_raw(joiner as u64)), 1_000).await.unwrap(), None);

        // 만료된 초대 → None.
        store
            .create_invite(&NewInvite {
                code: "EXPIRED1".into(),
                realm_id: realm,
                channel_id: None,
                inviter_id: None,
                max_uses: 0,
                expires_at: Some(500),
            })
            .await
            .unwrap();
        assert_eq!(store.redeem_invite("EXPIRED1", UserId(Snowflake::from_raw(joiner as u64)), 1_000).await.unwrap(), None);

        // 정리 (realm CASCADE → members/invites 삭제).
        sqlx::query("DELETE FROM realms WHERE id = $1").bind(realm.0.raw() as i64).execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM users WHERE id = ANY($1)").bind(vec![owner, joiner]).execute(&pool).await.unwrap();
    }
}
