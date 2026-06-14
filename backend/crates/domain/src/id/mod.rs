//! Snowflake ID + 엔티티 id 뉴타입 (개념: id).
//! 64bit = [timestamp 41 | worker 10 | sequence 12] (D11). 중앙 시퀀스 없음.

use core::fmt;
use core::sync::atomic::{AtomicU64, Ordering};

/// 커스텀 epoch (ms). 2023-11-14 근처.
pub const EPOCH_MS: u64 = 1_700_000_000_000;

const WORKER_BITS: u64 = 10;
const SEQ_BITS: u64 = 12;
const WORKER_MAX: u16 = (1 << WORKER_BITS) - 1;
const SEQ_MAX: u16 = (1 << SEQ_BITS) - 1;

/// 시간순 정렬 가능한 64bit 식별자.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Snowflake(u64);

impl Snowflake {
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
    pub const fn raw(self) -> u64 {
        self.0
    }
    pub const fn timestamp_ms(self) -> u64 {
        (self.0 >> (WORKER_BITS + SEQ_BITS)) + EPOCH_MS
    }
    pub const fn worker(self) -> u16 {
        ((self.0 >> SEQ_BITS) & WORKER_MAX as u64) as u16
    }
    pub const fn sequence(self) -> u16 {
        (self.0 & SEQ_MAX as u64) as u16
    }
}

impl fmt::Display for Snowflake {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Snowflake 생성기. `now_ms`를 **호출자가 주입** → DST 시 SimClock 주입 가능 (D25).
/// worker id는 클러스터 config에서 부여 (D29).
///
/// **불변식: `1 worker_id = 1 generator`** (D11). 전역 유일성이 여기 의존하므로
/// 노드당 정확히 1개만 만들어 모든 액터에 `Arc`로 **주입**한다 — 액터당 생성 금지.
/// 동시 접근되므로 thread-safe & lock-free: 가변 상태 `(last_ms_rel, seq)`를
/// `AtomicU64` 하나에 packing(상위=epoch 기준 ms, 하위 12비트=seq)하고 CAS로 갱신.
#[derive(Debug)]
pub struct SnowflakeGenerator {
    worker: u16,
    /// packed = (last_ms_rel << SEQ_BITS) | seq. last_ms_rel = ms - EPOCH_MS.
    state: AtomicU64,
}

impl SnowflakeGenerator {
    pub fn new(worker: u16) -> Self {
        assert!(worker <= WORKER_MAX, "worker id out of range (max {WORKER_MAX})");
        Self { worker, state: AtomicU64::new(0) }
    }

    /// 단조 증가 보장. 같은 ms 내 seq 소진 시 논리적으로 다음 ms로 진행.
    /// `&self` + CAS 루프 → 여러 액터가 동시 호출해도 안전·유일 (락 없음, D11).
    pub fn next(&self, now_ms: u64) -> Snowflake {
        let now_rel = now_ms.saturating_sub(EPOCH_MS);
        loop {
            let old = self.state.load(Ordering::Relaxed);
            let last_rel = old >> SEQ_BITS;
            let last_seq = old & SEQ_MAX as u64;

            let (new_rel, new_seq) = if now_rel > last_rel {
                // 새 ms → seq 리셋.
                (now_rel, 0)
            } else {
                // 같은 ms(또는 시계 역행) → seq 증가. 소진 시 논리적 다음 ms로.
                let s = last_seq + 1;
                if s > SEQ_MAX as u64 { (last_rel + 1, 0) } else { (last_rel, s) }
            };

            let packed = (new_rel << SEQ_BITS) | new_seq;
            if self
                .state
                .compare_exchange_weak(old, packed, Ordering::SeqCst, Ordering::Relaxed)
                .is_ok()
            {
                let raw = (new_rel << (WORKER_BITS + SEQ_BITS))
                    | ((self.worker as u64) << SEQ_BITS)
                    | new_seq;
                return Snowflake(raw);
            }
            // CAS 실패(경합) → 재시도.
        }
    }
}

/// 개념별 타입 안전 id 뉴타입 생성.
macro_rules! entity_id {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
        pub struct $name(pub Snowflake);

        impl core::fmt::Display for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
        impl From<Snowflake> for $name {
            fn from(s: Snowflake) -> Self {
                Self(s)
            }
        }
    };
}

entity_id!(
    /// 유저 id
    UserId
);
entity_id!(
    /// Realm(길드/DM/그룹DM 통일) id
    RealmId
);
entity_id!(
    /// 채널 id
    ChannelId
);
entity_id!(
    /// 메시지 id
    MessageId
);
entity_id!(
    /// 역할 id
    RoleId
);
entity_id!(
    /// refresh 토큰 id
    RefreshTokenId
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monotonic_within_same_ms() {
        let g = SnowflakeGenerator::new(7);
        let a = g.next(EPOCH_MS + 5);
        let b = g.next(EPOCH_MS + 5);
        assert!(b > a);
        assert_eq!(a.worker(), 7);
        assert_eq!(a.timestamp_ms(), EPOCH_MS + 5);
        assert_eq!(b.sequence(), a.sequence() + 1);
    }

    #[test]
    fn sequence_resets_on_new_ms() {
        let g = SnowflakeGenerator::new(1);
        let a = g.next(EPOCH_MS + 1);
        let b = g.next(EPOCH_MS + 2);
        assert!(b > a);
        assert_eq!(b.sequence(), 0);
    }

    /// 시계가 역행해도 단조성 유지 (CAS 분기: now_rel <= last_rel).
    #[test]
    fn backwards_clock_stays_monotonic() {
        let g = SnowflakeGenerator::new(3);
        let a = g.next(EPOCH_MS + 100);
        let b = g.next(EPOCH_MS + 50); // 과거로 역행
        assert!(b > a, "역행해도 ID는 단조 증가해야 함");
    }

    /// 정석 회귀 테스트: 같은 generator를 **여러 스레드가 동시 호출**해도
    /// 발급된 ID가 전부 유일 (D11 불변식 = 노드당 generator 1개의 핵심).
    #[test]
    fn concurrent_minting_is_globally_unique() {
        use std::collections::HashSet;
        use std::sync::Arc;

        let g = Arc::new(SnowflakeGenerator::new(5));
        let per_thread = 5_000;
        let threads = 8;

        let handles: Vec<_> = (0..threads)
            .map(|_| {
                let g = Arc::clone(&g);
                std::thread::spawn(move || {
                    // 같은 ms로 고정 → seq 경합을 최대화.
                    (0..per_thread).map(|_| g.next(EPOCH_MS + 1).raw()).collect::<Vec<_>>()
                })
            })
            .collect();

        let mut all = HashSet::new();
        for h in handles {
            for id in h.join().unwrap() {
                assert!(all.insert(id), "중복 ID 발급됨 — 유일성 불변식 위반");
            }
        }
        assert_eq!(all.len(), threads * per_thread);
    }
}
