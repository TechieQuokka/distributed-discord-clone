//! 메시지 파티션 유지보수 (개념: partition). D28, 04 §6.
//!
//! 도메인 port가 아니라 **운영(ops) 관심사** — `PgStore`의 인헌트 메서드로 둔다(domain 무관).
//! 달력 계산은 V19의 plpgsql 함수 `ensure_message_partitions`에 위임(앱에 날짜 라이브러리 무도입).
//! server가 startup에 1회 호출 → 이번 달 + 다가오는 N개월 파티션을 멱등 생성(이미 있으면 스킵).

use domain::repo::RepoError;

use crate::store::{PgStore, map_err};

impl PgStore {
    /// 이번 달부터 `months_ahead`개월 뒤까지의 메시지 파티션을 보장(멱등). 새로 만든 수를 반환.
    /// 미래 달은 DEFAULT에 행이 없어 안전(04 §6). 신규 월 메시지가 DEFAULT로 새는 것을 방지.
    pub async fn ensure_message_partitions(&self, months_ahead: i32) -> Result<i64, RepoError> {
        let created: i64 = sqlx::query_scalar("SELECT ensure_message_partitions($1)::bigint")
            .bind(months_ahead)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(created)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{connect, run_migrations};

    /// 실제 Postgres 필요 — `DATABASE_URL` 미설정 시 skip. 멱등성 검증:
    /// 1차 호출은 다가오는 달을 만들 수 있고(>=0), 2차 호출은 0(이미 전부 존재).
    #[tokio::test]
    async fn ensure_partitions_is_idempotent() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL 미설정 — partition 통합 테스트 skip");
            return;
        };
        let pool = connect(&url).await.expect("connect");
        run_migrations(&pool).await.expect("migrate");
        let store = PgStore::new(pool);

        let first = store.ensure_message_partitions(3).await.expect("first ensure");
        assert!(first >= 0, "생성 수는 음수가 아니어야");
        let second = store.ensure_message_partitions(3).await.expect("second ensure");
        assert_eq!(second, 0, "두 번째 호출은 모두 존재 → 0개 생성(멱등)");
    }
}
