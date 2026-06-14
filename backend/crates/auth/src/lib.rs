//! `auth` — 인증 프리미티브 (D14/D15). P6: 크립토는 검증된 크레이트(argon2/pasetors).
//!
//! 개념 모듈 분리 (CLAUDE.md R6): `password`(Argon2id), `token`(PASETO v4.public), `refresh`(opaque), `error`.

pub mod error;
pub mod password;
pub mod refresh;
pub mod token;

pub use error::AuthError;
pub use password::{hash_password, verify_password};
pub use refresh::{generate_refresh, hash_refresh};
pub use token::TokenKeys;
