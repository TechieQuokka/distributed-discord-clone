//! 노드 생사 판정 (개념: membership). PING/PONG 기반 failure detection (D23).
//!
//! 각 노드가 피어에게 주기적으로 PING → PONG(또는 임의 트래픽)으로 last_seen 갱신.
//! `timeout` 동안 소식 없으면 **down**으로 표시 → `Router::owner`가 링에서 그 노드를 건너뜀
//! → Realm 소유권이 다음 살아있는 노드로 자동 이동(rehydrate, D23). 재연결 시 다시 alive.
//!
//! 시간은 주입된 `Clock`(ms)로 다룬다 — DST/테스트에서 ManualClock으로 결정론 재현(D25).

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

#[derive(Default)]
struct State {
    /// node_id → 마지막으로 살아있음을 확인한 시각(ms).
    last_seen: HashMap<u64, u64>,
    /// 현재 down으로 판정된 노드.
    down: HashSet<u64>,
}

/// 피어 생사 뷰. Router가 `Arc`로 공유, failure detector 루프가 갱신.
#[derive(Default)]
pub struct Membership {
    inner: Mutex<State>,
}

impl Membership {
    pub fn new() -> Self {
        Self::default()
    }

    /// 피어로부터 트래픽 수신(PONG 포함) → 살아있음. down에서 해제.
    pub fn record_seen(&self, node: u64, now_ms: u64) {
        let mut s = self.inner.lock().unwrap();
        s.last_seen.insert(node, now_ms);
        s.down.remove(&node);
    }

    /// 명시적 down 표시 (연결 종료 통지 등 즉시 반영용).
    pub fn mark_down(&self, node: u64) {
        self.inner.lock().unwrap().down.insert(node);
    }

    pub fn is_down(&self, node: u64) -> bool {
        self.inner.lock().unwrap().down.contains(&node)
    }

    /// 현재 down 집합 스냅샷 (Router::owner가 링 탐색에서 제외).
    pub fn down_set(&self) -> HashSet<u64> {
        self.inner.lock().unwrap().down.clone()
    }

    /// 주기 호출: `peers` 중 last_seen이 `timeout_ms`보다 오래된(또는 한 번도 못 본) 노드를 down 처리.
    /// 최근 본 노드는 down에서 해제. detector가 PING 후 호출.
    pub fn sweep(&self, peers: &[u64], now_ms: u64, timeout_ms: u64) {
        let mut s = self.inner.lock().unwrap();
        for &p in peers {
            let stale = match s.last_seen.get(&p) {
                Some(&t) => now_ms.saturating_sub(t) > timeout_ms,
                None => true, // 한 번도 못 봄 → down 취급(detector 시작 시 seed로 완화).
            };
            if stale {
                s.down.insert(p);
            } else {
                s.down.remove(&p);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unseen_peer_swept_down_then_recovers() {
        let m = Membership::new();
        // t=1000, peer 2를 한 번도 못 봄 → down.
        m.sweep(&[2], 1000, 3000);
        assert!(m.is_down(2));

        // PONG 수신 → alive.
        m.record_seen(2, 1500);
        assert!(!m.is_down(2));

        // 계속 신선하면 down 아님.
        m.sweep(&[2], 2000, 3000);
        assert!(!m.is_down(2));

        // timeout 초과 → 다시 down.
        m.sweep(&[2], 1500 + 3001, 3000);
        assert!(m.is_down(2));
    }

    #[test]
    fn record_seen_clears_down() {
        let m = Membership::new();
        m.mark_down(7);
        assert!(m.is_down(7));
        m.record_seen(7, 10);
        assert!(!m.is_down(7));
        assert_eq!(m.down_set().len(), 0);
    }
}
