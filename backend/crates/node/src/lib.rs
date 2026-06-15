//! `node` — Realm 액터 배선 + consistent hashing 라우팅 (D6/D7/D9).
//!
//! 개념 모듈 분리 (CLAUDE.md R6):
//! - `ring`  — consistent hashing (Realm → 소유 노드 배치, D6)
//! - `realm` — Realm 액터(단일 소유 순서보장 D24, 구독자표 D12)
//! - `clock` — 시간 주입(DST D25)
//!
//! 후속: 2단 라우팅(세션 소유 vs Realm 소유, D9) = ring + transport 결합.

pub mod clock;
pub mod membership;
pub mod presence;
pub mod realm;
pub mod ring;
pub mod router;

pub use clock::{Clock, ManualClock, SystemClock};
pub use membership::Membership;
pub use presence::{Presence, Status};
pub use realm::{RealmActor, RealmCommand, RealmEvent};
pub use ring::HashRing;
pub use router::{LocalDelivery, Routed, Router, RouterError, run_failure_detector};
