//! 시간 주입 (개념: clock). Snowflake 등에서 사용. DST 시 ManualClock 주입 (D25).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub trait Clock: Send + Sync + 'static {
    fn now_ms(&self) -> u64;
}

/// 실제 시스템 시계.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before epoch")
            .as_millis() as u64
    }
}

/// 수동 시계 — 테스트/DST에서 시간을 직접 제어 (D25).
pub struct ManualClock(AtomicU64);

impl ManualClock {
    pub fn new(start_ms: u64) -> Self {
        Self(AtomicU64::new(start_ms))
    }
    pub fn set(&self, ms: u64) {
        self.0.store(ms, Ordering::SeqCst);
    }
    pub fn advance(&self, delta_ms: u64) {
        self.0.fetch_add(delta_ms, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}
