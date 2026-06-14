//! 통합 Postgres 저장소 (개념: store). 모든 domain port를 한 타입이 구현 (D22).
//!
//! `PgStore` 하나가 `User/RefreshToken/Guild/Channel/Message` 리포지토리를 모두 구현 →
//! 조합 루트(server·rest-api)는 `Arc<PgStore>` 제네릭 1개로 주입(제네릭 폭발 방지).
//! 각 port 구현은 개념 모듈(user/refresh_token/guild/channel/message)에 분산.

use sqlx::PgPool;

#[derive(Clone)]
pub struct PgStore {
    pub(crate) pool: PgPool,
}

impl PgStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// sqlx 에러 → 도메인 RepoError. 유니크 위반만 Conflict, 나머진 Backend.
pub(crate) fn map_err(e: sqlx::Error) -> domain::repo::RepoError {
    match &e {
        sqlx::Error::Database(db) if db.is_unique_violation() => domain::repo::RepoError::Conflict,
        _ => domain::repo::RepoError::Backend(e.to_string()),
    }
}
