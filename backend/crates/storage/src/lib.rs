//! `storage` — Postgres 리포지토리 (domain port의 adapter, D22/D28).
//!
//! 개념 모듈 분리 (CLAUDE.md R6): `pool`(연결/마이그레이션).
//! 리포지토리 구현(user/realm/message 등)은 엔티티별로 추가 — domain의 trait(port)를 구현.

pub mod channel;
pub mod guild;
pub mod invite;
pub mod message;
pub mod pool;
pub mod refresh_token;
pub mod store;
pub mod user;

pub use pool::{connect, run_migrations};
pub use sqlx::PgPool;
pub use store::PgStore;
