//! `actor-rt` — 수제 액터 런타임 (D7). tokio + mpsc, 도메인 무지(범용).
//!
//! 개념 단위 모듈 분리 (CLAUDE.md R6): `actor`(trait+spawn), `mailbox`(주소+백프레셔).

pub mod actor;
pub mod mailbox;

pub use actor::{Actor, spawn};
pub use mailbox::{Mailbox, SendError};
