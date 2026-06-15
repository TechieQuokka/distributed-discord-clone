//! `domain` — 코어 엔티티 + 리포지토리 trait(port). IO 무의존, 헥사고날 중심 (D22).
//!
//! 개념 단위 디렉터리/모듈 분리 (CLAUDE.md R6).
//! 엔티티(user/guild/channel/message/role/member/realm 등)는 구현되며 각자 디렉터리로 추가된다.

pub mod channel;
pub mod dm;
pub mod emit;
pub mod error;
pub mod guild;
pub mod id;
pub mod invite;
pub mod member;
pub mod mention;
pub mod message;
pub mod permissions;
pub mod read_state;
pub mod refresh_token;
pub mod relationship;
pub mod repo;
pub mod role;
pub mod user;
