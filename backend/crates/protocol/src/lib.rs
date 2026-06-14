//! `protocol` — 노드 간 와이어 타입 + 수제 바이트 코덱 (D3/D36). 명세: `docs/protocol/node-wire.md`.
//!
//! 개념 단위 모듈 분리 (CLAUDE.md R6):
//! - `codec`  — 원시 타입 인코딩(빅엔디언/길이접두사)
//! - `frame`  — 28바이트 헤더 + 길이접두사 프레이밍
//! - `message`— `NodeMessage` enum + 본문 코덱

pub mod codec;
pub mod frame;
pub mod message;

pub use codec::{DecodeError, Reader, Writer};
pub use frame::{Header, MAX_FRAME, PROTOCOL_VERSION, encode_frame, read_frame};
pub use message::{NodeMessage, msg_type};
