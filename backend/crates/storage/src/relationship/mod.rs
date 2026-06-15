//! `RelationshipRepository` 구현 for `PgStore` (개념: relationship). adapter (D22).
//!
//! 방향성 행(02-schema §6). 상태 전이는 두 행을 함께 바꾸므로 트랜잭션. enum은 `$n::relation_kind` 캐스트.

use domain::id::{Snowflake, UserId};
use domain::relationship::{RelationKind, Relationship};
use domain::repo::{RelationshipRepository, RepoError};
use sqlx::Row;

use crate::store::{PgStore, map_err};

/// (user,target) 한 방향 행의 kind를 트랜잭션 안에서 조회.
async fn get_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user: i64,
    target: i64,
) -> Result<Option<RelationKind>, RepoError> {
    let row = sqlx::query("SELECT kind::text AS kind FROM relationships WHERE user_id = $1 AND target_id = $2")
        .bind(user)
        .bind(target)
        .fetch_optional(&mut **tx)
        .await
        .map_err(map_err)?;
    Ok(row.and_then(|r| RelationKind::parse(&r.get::<String, _>("kind"))))
}

/// 한 방향 행 upsert.
async fn upsert_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user: i64,
    target: i64,
    kind: RelationKind,
) -> Result<(), RepoError> {
    sqlx::query(
        "INSERT INTO relationships (user_id, target_id, kind) VALUES ($1, $2, $3::relation_kind)
         ON CONFLICT (user_id, target_id) DO UPDATE SET kind = EXCLUDED.kind",
    )
    .bind(user)
    .bind(target)
    .bind(kind.as_str())
    .execute(&mut **tx)
    .await
    .map_err(map_err)?;
    Ok(())
}

async fn delete_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user: i64,
    target: i64,
) -> Result<(), RepoError> {
    sqlx::query("DELETE FROM relationships WHERE user_id = $1 AND target_id = $2")
        .bind(user)
        .bind(target)
        .execute(&mut **tx)
        .await
        .map_err(map_err)?;
    Ok(())
}

impl RelationshipRepository for PgStore {
    async fn list_relationships(&self, user: UserId) -> Result<Vec<Relationship>, RepoError> {
        let rows = sqlx::query(
            "SELECT target_id, kind::text AS kind FROM relationships WHERE user_id = $1 ORDER BY created_at",
        )
        .bind(user.0.raw() as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(rows
            .iter()
            .filter_map(|r| {
                RelationKind::parse(&r.get::<String, _>("kind")).map(|kind| Relationship {
                    user_id: user,
                    target_id: UserId(Snowflake::from_raw(r.get::<i64, _>("target_id") as u64)),
                    kind,
                })
            })
            .collect())
    }

    async fn get_relationship(
        &self,
        user: UserId,
        target: UserId,
    ) -> Result<Option<RelationKind>, RepoError> {
        let mut tx = self.pool.begin().await.map_err(map_err)?;
        let k = get_tx(&mut tx, user.0.raw() as i64, target.0.raw() as i64).await?;
        tx.commit().await.map_err(map_err)?;
        Ok(k)
    }

    async fn is_blocked_between(&self, a: UserId, b: UserId) -> Result<bool, RepoError> {
        let row = sqlx::query(
            "SELECT 1 AS x FROM relationships
             WHERE kind = 'blocked'
               AND ((user_id = $1 AND target_id = $2) OR (user_id = $2 AND target_id = $1))
             LIMIT 1",
        )
        .bind(a.0.raw() as i64)
        .bind(b.0.raw() as i64)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(row.is_some())
    }

    async fn friend_request_or_accept(
        &self,
        me: UserId,
        target: UserId,
    ) -> Result<RelationKind, RepoError> {
        let (m, t) = (me.0.raw() as i64, target.0.raw() as i64);
        let mut tx = self.pool.begin().await.map_err(map_err)?;
        let mine = get_tx(&mut tx, m, t).await?;
        let result = match mine {
            Some(RelationKind::Friend) => RelationKind::Friend,
            Some(RelationKind::PendingOut) => RelationKind::PendingOut,
            // 상대가 먼저 요청(내 행 PendingIn) → 수락: 양쪽 Friend.
            Some(RelationKind::PendingIn) => {
                upsert_tx(&mut tx, m, t, RelationKind::Friend).await?;
                upsert_tx(&mut tx, t, m, RelationKind::Friend).await?;
                RelationKind::Friend
            }
            // 차단 상태에서 호출되면 안 됨(라우트가 막음) — 방어적으로 그대로 둠.
            Some(RelationKind::Blocked) => RelationKind::Blocked,
            // 관계 없음 → 요청 생성.
            None => {
                upsert_tx(&mut tx, m, t, RelationKind::PendingOut).await?;
                upsert_tx(&mut tx, t, m, RelationKind::PendingIn).await?;
                RelationKind::PendingOut
            }
        };
        tx.commit().await.map_err(map_err)?;
        Ok(result)
    }

    async fn block(&self, me: UserId, target: UserId) -> Result<(), RepoError> {
        let (m, t) = (me.0.raw() as i64, target.0.raw() as i64);
        let mut tx = self.pool.begin().await.map_err(map_err)?;
        upsert_tx(&mut tx, m, t, RelationKind::Blocked).await?;
        delete_tx(&mut tx, t, m).await?; // 상대 쪽 친구/대기 관계 해제.
        tx.commit().await.map_err(map_err)?;
        Ok(())
    }

    async fn remove_relationship(
        &self,
        me: UserId,
        target: UserId,
    ) -> Result<Option<RelationKind>, RepoError> {
        let (m, t) = (me.0.raw() as i64, target.0.raw() as i64);
        let mut tx = self.pool.begin().await.map_err(map_err)?;
        let mine = get_tx(&mut tx, m, t).await?;
        match mine {
            None => {}
            Some(RelationKind::Blocked) => delete_tx(&mut tx, m, t).await?, // 차단 해제 — 내 행만.
            Some(_) => {
                // 친구/대기 해제 — 양쪽 행 제거.
                delete_tx(&mut tx, m, t).await?;
                delete_tx(&mut tx, t, m).await?;
            }
        }
        tx.commit().await.map_err(map_err)?;
        Ok(mine)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PgStore, connect, run_migrations};

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

    /// 실제 Postgres 필요 — skip if no DATABASE_URL. 친구 요청→수락→제거, 차단→해제.
    #[tokio::test]
    async fn friend_block_lifecycle() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL 미설정 — relationship 통합 테스트 skip");
            return;
        };
        let pool = connect(&url).await.expect("connect");
        run_migrations(&pool).await.expect("migrate");
        let store = PgStore::new(pool.clone());

        let (a, b) = (9_500_000_001i64, 9_500_000_002i64);
        mk_user(&pool, a, "rel_a").await;
        mk_user(&pool, b, "rel_b").await;
        let (ua, ub) = (UserId(Snowflake::from_raw(a as u64)), UserId(Snowflake::from_raw(b as u64)));
        sqlx::query("DELETE FROM relationships WHERE user_id = ANY($1) OR target_id = ANY($1)")
            .bind(vec![a, b]).execute(&pool).await.unwrap();

        // a→b 친구 요청 → PendingOut / b쪽 PendingIn.
        assert_eq!(store.friend_request_or_accept(ua, ub).await.unwrap(), RelationKind::PendingOut);
        assert_eq!(store.get_relationship(ua, ub).await.unwrap(), Some(RelationKind::PendingOut));
        assert_eq!(store.get_relationship(ub, ua).await.unwrap(), Some(RelationKind::PendingIn));
        assert!(!store.is_blocked_between(ua, ub).await.unwrap());

        // b가 수락(b 입장에서 친구 요청/수락) → 양쪽 Friend.
        assert_eq!(store.friend_request_or_accept(ub, ua).await.unwrap(), RelationKind::Friend);
        assert_eq!(store.get_relationship(ua, ub).await.unwrap(), Some(RelationKind::Friend));
        assert_eq!(store.list_relationships(ua).await.unwrap().len(), 1);

        // a가 친구 제거 → 양쪽 행 사라짐.
        assert_eq!(store.remove_relationship(ua, ub).await.unwrap(), Some(RelationKind::Friend));
        assert!(store.get_relationship(ua, ub).await.unwrap().is_none());
        assert!(store.get_relationship(ub, ua).await.unwrap().is_none());

        // a가 b 차단 → a행 Blocked, b행 없음, is_blocked_between true.
        store.block(ua, ub).await.unwrap();
        assert_eq!(store.get_relationship(ua, ub).await.unwrap(), Some(RelationKind::Blocked));
        assert!(store.get_relationship(ub, ua).await.unwrap().is_none());
        assert!(store.is_blocked_between(ub, ua).await.unwrap());

        // 차단 해제 → 내 행만 제거(상대 영향 없음).
        assert_eq!(store.remove_relationship(ua, ub).await.unwrap(), Some(RelationKind::Blocked));
        assert!(!store.is_blocked_between(ua, ub).await.unwrap());

        sqlx::query("DELETE FROM users WHERE id = ANY($1)").bind(vec![a, b]).execute(&pool).await.unwrap();
    }
}
