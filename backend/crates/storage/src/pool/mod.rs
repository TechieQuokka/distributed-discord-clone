//! Postgres 연결 풀 + 마이그레이션 (개념: pool). D28(sqlx).

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

/// 연결 풀 생성. `database_url`은 환경변수(.env)에서 주입 — 코드 하드코딩 금지 (D20).
pub async fn connect(database_url: &str) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(10)
        .connect(database_url)
        .await
}

/// `./migrations` 임베드 마이그레이션 적용 (D28).
pub async fn run_migrations(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("./migrations").run(pool).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 실제 Postgres 필요 — `DATABASE_URL` 미설정 시 skip.
    #[tokio::test]
    async fn migrate_and_user_round_trip() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL 미설정 — storage 통합 테스트 skip");
            return;
        };
        let pool = connect(&url).await.expect("connect");
        run_migrations(&pool).await.expect("migrate");

        let id: i64 = 123_456_789;
        sqlx::query(
            "INSERT INTO users (id, username, email, password_hash)
             VALUES ($1, $2, $3, $4) ON CONFLICT (id) DO NOTHING",
        )
        .bind(id)
        .bind("alice")
        .bind("alice@example.com")
        .bind("hash")
        .execute(&pool)
        .await
        .expect("insert user");

        let (name,): (String,) = sqlx::query_as("SELECT username FROM users WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("select user");
        assert_eq!(name, "alice");

        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .expect("cleanup");
    }
}
