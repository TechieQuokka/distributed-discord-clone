//! `DmRepository` 구현 for `PgStore` (개념: dm). adapter (D22, DB-D2).
//!
//! Realm 통일(D8): DM도 realms + channels(+ members) — 길드 생성과 같은 구조의 트랜잭션.
//! 1:1 DM은 `dm_pairs`로 중복 방지. 멤버 추가/제거는 GuildRepository를 재사용한다.

use domain::dm::{DmChannel, NewDm, NewGroupDm, RealmInfo, RealmKind, order_pair};
use domain::id::{ChannelId, RealmId, Snowflake, UserId};
use domain::repo::{DmRepository, RepoError};
use sqlx::Row;

use crate::store::{PgStore, map_err};

impl DmRepository for PgStore {
    async fn find_dm(&self, a: UserId, b: UserId) -> Result<Option<DmChannel>, RepoError> {
        let (lo, hi) = order_pair(a, b);
        let row = sqlx::query(
            "SELECT dp.realm_id, c.id AS channel_id
             FROM dm_pairs dp
             JOIN channels c ON c.realm_id = dp.realm_id AND c.deleted_at IS NULL
             WHERE dp.user_lo = $1 AND dp.user_hi = $2
             LIMIT 1",
        )
        .bind(lo.0.raw() as i64)
        .bind(hi.0.raw() as i64)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(row.map(|r| DmChannel {
            realm_id: RealmId(Snowflake::from_raw(r.get::<i64, _>("realm_id") as u64)),
            channel_id: ChannelId(Snowflake::from_raw(r.get::<i64, _>("channel_id") as u64)),
            kind: RealmKind::Dm,
        }))
    }

    async fn create_dm(&self, dm: &NewDm) -> Result<(), RepoError> {
        let (lo, hi) = order_pair(dm.user_a, dm.user_b);
        let mut tx = self.pool.begin().await.map_err(map_err)?;

        sqlx::query("INSERT INTO realms (id, kind) VALUES ($1, 'dm')")
            .bind(dm.realm_id.0.raw() as i64)
            .execute(&mut *tx)
            .await
            .map_err(map_err)?;

        sqlx::query("INSERT INTO channels (id, realm_id, kind) VALUES ($1, $2, 'dm')")
            .bind(dm.channel_id.0.raw() as i64)
            .bind(dm.realm_id.0.raw() as i64)
            .execute(&mut *tx)
            .await
            .map_err(map_err)?;

        sqlx::query("INSERT INTO members (realm_id, user_id) VALUES ($1, $2), ($1, $3)")
            .bind(dm.realm_id.0.raw() as i64)
            .bind(dm.user_a.0.raw() as i64)
            .bind(dm.user_b.0.raw() as i64)
            .execute(&mut *tx)
            .await
            .map_err(map_err)?;

        // dm_pairs PK 유니크 — 레이스로 이미 있으면 여기서 Conflict(호출측이 find_dm 재조회).
        sqlx::query("INSERT INTO dm_pairs (user_lo, user_hi, realm_id) VALUES ($1, $2, $3)")
            .bind(lo.0.raw() as i64)
            .bind(hi.0.raw() as i64)
            .bind(dm.realm_id.0.raw() as i64)
            .execute(&mut *tx)
            .await
            .map_err(map_err)?;

        tx.commit().await.map_err(map_err)?;
        Ok(())
    }

    async fn create_group_dm(&self, dm: &NewGroupDm) -> Result<(), RepoError> {
        let mut tx = self.pool.begin().await.map_err(map_err)?;

        sqlx::query("INSERT INTO realms (id, kind, name, owner_id) VALUES ($1, 'group_dm', $2, $3)")
            .bind(dm.realm_id.0.raw() as i64)
            .bind(&dm.name)
            .bind(dm.owner.0.raw() as i64)
            .execute(&mut *tx)
            .await
            .map_err(map_err)?;

        sqlx::query("INSERT INTO channels (id, realm_id, kind, name) VALUES ($1, $2, 'dm', $3)")
            .bind(dm.channel_id.0.raw() as i64)
            .bind(dm.realm_id.0.raw() as i64)
            .bind(&dm.name)
            .execute(&mut *tx)
            .await
            .map_err(map_err)?;

        for m in &dm.members {
            sqlx::query("INSERT INTO members (realm_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
                .bind(dm.realm_id.0.raw() as i64)
                .bind(m.0.raw() as i64)
                .execute(&mut *tx)
                .await
                .map_err(map_err)?;
        }

        tx.commit().await.map_err(map_err)?;
        Ok(())
    }

    async fn get_realm(&self, realm_id: RealmId) -> Result<Option<RealmInfo>, RepoError> {
        let row = sqlx::query("SELECT kind::text AS kind, owner_id, name FROM realms WHERE id = $1")
            .bind(realm_id.0.raw() as i64)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(row.map(|r| RealmInfo {
            id: realm_id,
            kind: RealmKind::parse(&r.get::<String, _>("kind")).unwrap_or(RealmKind::Guild),
            owner_id: r
                .get::<Option<i64>, _>("owner_id")
                .map(|o| UserId(Snowflake::from_raw(o as u64))),
            name: r.get("name"),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PgStore, connect, run_migrations};
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

    /// 실제 Postgres 필요 — `DATABASE_URL` 미설정 시 skip. 1:1 DM find-or-create 멱등 + 그룹DM.
    #[tokio::test]
    async fn dm_find_or_create_and_group() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL 미설정 — dm 통합 테스트 skip");
            return;
        };
        let pool = connect(&url).await.expect("connect");
        run_migrations(&pool).await.expect("migrate");
        let store = PgStore::new(pool.clone());

        let (a, b, c) = (9_400_000_001i64, 9_400_000_002i64, 9_400_000_003i64);
        let realm = RealmId(Snowflake::from_raw(9_400_000_010));
        let chan = ChannelId(Snowflake::from_raw(9_400_000_011));
        let grealm = RealmId(Snowflake::from_raw(9_400_000_020));
        let gchan = ChannelId(Snowflake::from_raw(9_400_000_021));
        for (id, n) in [(a, "dm_a"), (b, "dm_b"), (c, "dm_c")] {
            mk_user(&pool, id, n).await;
        }
        for r in [realm, grealm] {
            sqlx::query("DELETE FROM realms WHERE id = $1").bind(r.0.raw() as i64).execute(&pool).await.unwrap();
        }
        let (ua, ub, uc) = (
            UserId(Snowflake::from_raw(a as u64)),
            UserId(Snowflake::from_raw(b as u64)),
            UserId(Snowflake::from_raw(c as u64)),
        );

        // 처음엔 없음.
        assert!(store.find_dm(ua, ub).await.unwrap().is_none());

        // 생성 → 양방향 조회로 같은 채널.
        store.create_dm(&NewDm { realm_id: realm, channel_id: chan, user_a: ua, user_b: ub }).await.unwrap();
        let found = store.find_dm(ub, ua).await.unwrap().expect("DM 있어야");
        assert_eq!(found.realm_id, realm);
        assert_eq!(found.channel_id, chan);
        assert!(store.is_member(realm, ua).await.unwrap());
        assert!(store.is_member(realm, ub).await.unwrap());

        // 재생성 시도 → dm_pairs PK 충돌 = Conflict.
        let again = store
            .create_dm(&NewDm {
                realm_id: RealmId(Snowflake::from_raw(9_400_000_099)),
                channel_id: ChannelId(Snowflake::from_raw(9_400_000_098)),
                user_a: ua,
                user_b: ub,
            })
            .await;
        assert!(matches!(again, Err(RepoError::Conflict)));

        // 그룹DM: owner=a, members a,b,c.
        store
            .create_group_dm(&NewGroupDm {
                realm_id: grealm,
                channel_id: gchan,
                owner: ua,
                name: Some("squad".into()),
                members: vec![ua, ub, uc],
            })
            .await
            .unwrap();
        let info = store.get_realm(grealm).await.unwrap().expect("realm");
        assert_eq!(info.kind, RealmKind::GroupDm);
        assert_eq!(info.owner_id, Some(ua));
        assert_eq!(info.name.as_deref(), Some("squad"));
        assert_eq!(store.list_members(grealm).await.unwrap().len(), 3);

        // 정리.
        for r in [realm, grealm] {
            sqlx::query("DELETE FROM realms WHERE id = $1").bind(r.0.raw() as i64).execute(&pool).await.unwrap();
        }
        sqlx::query("DELETE FROM users WHERE id = ANY($1)").bind(vec![a, b, c]).execute(&pool).await.unwrap();
    }
}
