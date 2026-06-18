//! `gateway` — 실시간 WS 계층 (D2/D13/D24). 클라 엣지는 JSON(D31).
//!
//! 개념 모듈 분리 (CLAUDE.md R6):
//! - `protocol` — JSON op 프레임 (gateway.md)
//! - `hub`      — 노드 로컬 세션 레지스트리 + 팬아웃 배달 (D9 세션 소유)
//! - `session`  — 연결 수명주기 (HELLO/IDENTIFY/READY/HEARTBEAT/DISPATCH)
//! - `dispatch` — Realm 이벤트 → persist → fanout → 세션 배달 (D24)
//! - `user_route` — 유저 단위 이벤트의 크로스노드 라우팅 (D43, `UserEmitter` 구현)
//! - `state`    — 공유 상태 (Router/transport 제네릭 격리)
//! - `routes`   — WS 업그레이드 + 메시지 전송(REST)

pub mod dispatch;
pub mod hub;
pub mod presence;
pub mod protocol;
pub mod routes;
pub mod session;
pub mod state;
pub mod user_route;

pub use dispatch::{deliver_local, run_dispatch};
pub use hub::{Hub, MigratedSession, MigrationExport, ResumeOutcome};
pub use routes::router;
pub use state::GatewayState;
pub use user_route::{UserRouter, deliver_user};
