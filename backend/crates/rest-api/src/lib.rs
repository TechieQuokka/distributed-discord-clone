//! `rest-api` — HTTP REST 계층 (inbound 어댑터). axum 기반.
//!
//! 개념 모듈 분리 (CLAUDE.md R6): `state`(공유 상태), `routes`(핸들러), `error`(HTTP 매핑).
//! 인증 흐름은 D14(2-토큰)/D15(Argon2id)를 따른다.

pub mod error;
pub mod events;
pub mod extract;
pub mod perm;
pub mod ratelimit;
pub mod routes;
pub mod state;

pub use error::ApiError;
pub use ratelimit::RateLimiter;
pub use routes::router;
pub use state::AppState;
