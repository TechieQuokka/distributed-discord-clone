//! `CrdtRepository` 구현 for `PgStore` (개념: crdt). CRDT 오프라인 동기화 adapter (D49).
//!
//! 유저 동기화 문서 = 키별 LWW. 병합 권위는 domain `LwwMap`이고, 어댑터는 **LWW 가드 upsert**
//! ((ts,node) 큰 것만 채택)로 영속 → DB가 LWW 시맨틱을 보존. 여러 기기 동시 push도 수렴.

use domain::crdt::{CrdtEntry, LwwMap};
use domain::id::UserId;
use domain::repo::{CrdtRepository, RepoError};
use sqlx::Row;

use crate::store::{PgStore, map_err};

impl CrdtRepository for PgStore {
    async fn load_user_doc(&self, user: UserId) -> Result<LwwMap, RepoError> {
        let rows = sqlx::query("SELECT key, value, ts_ms, node_id FROM user_crdt_entries WHERE user_id = $1")
            .bind(user.0.raw() as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)?;
        let entries = rows.iter().map(|r| {
            let ts: i64 = r.get("ts_ms");
            let node: i64 = r.get("node_id");
            CrdtEntry { key: r.get("key"), value: r.get("value"), ts_ms: ts as u64, node: node as u64 }
        });
        Ok(LwwMap::from_entries(entries))
    }

    async fn merge_user_doc(
        &self,
        user: UserId,
        entries: &[CrdtEntry],
    ) -> Result<LwwMap, RepoError> {
        let mut tx = self.pool.begin().await.map_err(map_err)?;
        for e in entries {
            // LWW 가드: 들어온 dot이 기존보다 클 때만 덮음. 행 비교 (a,b) > (c,d) = 사전식.
            sqlx::query(
                "INSERT INTO user_crdt_entries (user_id, key, value, ts_ms, node_id) \
                 VALUES ($1, $2, $3, $4, $5) \
                 ON CONFLICT (user_id, key) DO UPDATE \
                   SET value = EXCLUDED.value, ts_ms = EXCLUDED.ts_ms, node_id = EXCLUDED.node_id \
                   WHERE (EXCLUDED.ts_ms, EXCLUDED.node_id) \
                       > (user_crdt_entries.ts_ms, user_crdt_entries.node_id)",
            )
            .bind(user.0.raw() as i64)
            .bind(&e.key)
            .bind(&e.value)
            .bind(e.ts_ms as i64)
            .bind(e.node as i64)
            .execute(&mut *tx)
            .await
            .map_err(map_err)?;
        }
        tx.commit().await.map_err(map_err)?;
        self.load_user_doc(user).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{connect, run_migrations};
    use domain::id::Snowflake;

    fn uid(n: u64) -> UserId {
        UserId(Snowflake::from_raw(n))
    }
    fn entry(key: &str, value: Option<&str>, ts: u64, node: u64) -> CrdtEntry {
        CrdtEntry { key: key.into(), value: value.map(|s| s.into()), ts_ms: ts, node }
    }

    /// 실제 Postgres 필요 — `DATABASE_URL` 미설정 시 skip. 오프라인 두 기기가 같은 키를 편집한 뒤
    /// 각자 push → LWW로 수렴(더 늦은 쓰기 채택). 멱등 재push·툼스톤 삭제도 검증.
    #[tokio::test]
    async fn two_devices_converge_via_lww_merge() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL 미설정 — crdt 통합 테스트 skip");
            return;
        };
        let pool = connect(&url).await.expect("connect");
        run_migrations(&pool).await.expect("migrate");
        let store = PgStore::new(pool);

        // 유저 시드(FK 충족) + 깨끗한 시작.
        let u = uid(0xC2D7_0000_0001);
        sqlx::query("INSERT INTO users (id, username, email, password_hash) VALUES ($1,$2,$3,'h') ON CONFLICT (id) DO NOTHING")
            .bind(u.0.raw() as i64).bind("crdtuser").bind("crdt@e.com")
            .execute(&store.pool).await.unwrap();
        sqlx::query("DELETE FROM user_crdt_entries WHERE user_id = $1").bind(u.0.raw() as i64).execute(&store.pool).await.unwrap();

        // 기기1(node 1): draft="phone" @200. 기기2(node 2): draft="laptop" @210.
        store.merge_user_doc(u, &[entry("draft", Some("phone"), 200, 1)]).await.unwrap();
        let merged = store.merge_user_doc(u, &[entry("draft", Some("laptop"), 210, 2)]).await.unwrap();
        assert_eq!(merged.get("draft"), Some("laptop"), "더 늦은 쓰기가 이김(LWW)");

        // 더 이른 쓰기를 다시 push → 무시(수렴 안정성).
        let after = store.merge_user_doc(u, &[entry("draft", Some("phone"), 200, 1)]).await.unwrap();
        assert_eq!(after.get("draft"), Some("laptop"), "이른 쓰기는 못 덮음");

        // 멱등: 같은 엔트리 재push해도 불변.
        let again = store.merge_user_doc(u, &[entry("draft", Some("laptop"), 210, 2)]).await.unwrap();
        assert_eq!(again.get("draft"), Some("laptop"));

        // 툼스톤 삭제(더 늦은 dot) → live에서 사라짐.
        let deleted = store.merge_user_doc(u, &[entry("draft", None, 220, 2)]).await.unwrap();
        assert_eq!(deleted.get("draft"), None, "툼스톤 삭제");
        assert!(deleted.live().is_empty());

        sqlx::query("DELETE FROM users WHERE id = $1").bind(u.0.raw() as i64).execute(&store.pool).await.unwrap();
    }
}
