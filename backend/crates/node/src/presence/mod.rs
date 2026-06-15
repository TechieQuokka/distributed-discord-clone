//! 전역 presence 레지스트리 (개념: presence). Q11/D12 — Realm 무관 전역 유저 상태.
//!
//! **휘발 상태(DB-D5)**: 인메모리만. 노드 mesh의 gossip(`PRESENCE_GOSSIP`)로 전파된다.
//! 한 유저는 여러 노드에 세션을 가질 수 있으므로 user → (status, 그를 호스팅하는 노드 집합)으로
//! 추적: 노드 집합이 비면 offline(어느 노드에도 세션 없음). "online if any node hosts" — 멀티노드 정확.
//!
//! Membership(피어 생사)과 별개의 노드 레벨 상태 — server가 소유해 gateway·inbound 루프에 주입.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

/// 유저 상태 (DB `user_status` enum 대응). 현재 구현은 online/offline만 사용(idle/dnd는 후속 op 3).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Offline,
    Online,
    Idle,
    Dnd,
}

impl Status {
    pub fn as_u8(self) -> u8 {
        match self {
            Status::Offline => 0,
            Status::Online => 1,
            Status::Idle => 2,
            Status::Dnd => 3,
        }
    }
    pub fn from_u8(b: u8) -> Status {
        match b {
            1 => Status::Online,
            2 => Status::Idle,
            3 => Status::Dnd,
            _ => Status::Offline,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Offline => "offline",
            Status::Online => "online",
            Status::Idle => "idle",
            Status::Dnd => "dnd",
        }
    }
}

struct Entry {
    status: Status,
    /// 이 유저의 활성 세션을 가진 노드들. 비면 offline.
    nodes: HashSet<u64>,
}

impl Entry {
    fn effective(&self) -> Status {
        if self.nodes.is_empty() { Status::Offline } else { self.status }
    }
}

/// user → presence. 모든 노드가 자기 view를 들고, gossip으로 수렴(델타 전파).
#[derive(Default)]
pub struct Presence {
    inner: Mutex<HashMap<u64, Entry>>,
}

impl Presence {
    pub fn new() -> Self {
        Self::default()
    }

    /// user가 `node`에서 `status`로 온라인. **유효 상태가 바뀌면 true**(전이 시에만 통지·gossip).
    pub fn set(&self, user: u64, node: u64, status: Status) -> bool {
        let mut g = self.inner.lock().unwrap();
        let e = g.entry(user).or_insert_with(|| Entry { status: Status::Offline, nodes: HashSet::new() });
        let was = e.effective();
        e.nodes.insert(node);
        e.status = status;
        was != e.effective()
    }

    /// user가 `node`에서 사라짐(세션 종료). 노드 집합이 비면 offline. 유효 상태 변화 시 true.
    pub fn clear(&self, user: u64, node: u64) -> bool {
        let mut g = self.inner.lock().unwrap();
        let Some(e) = g.get_mut(&user) else { return false };
        let was = e.effective();
        e.nodes.remove(&node);
        let now = e.effective();
        if e.nodes.is_empty() {
            g.remove(&user);
        }
        was != now
    }

    pub fn get(&self, user: u64) -> Status {
        self.inner.lock().unwrap().get(&user).map(|e| e.effective()).unwrap_or(Status::Offline)
    }

    /// 이 유저를 호스팅하는 노드 집합 (D43 크로스노드 유저 라우팅 디렉터리).
    /// 오프라인(어느 노드에도 세션 없음)이면 빈 Vec. 유저 위치 조회의 단일 출처.
    pub fn nodes_for(&self, user: u64) -> Vec<u64> {
        self.inner
            .lock()
            .unwrap()
            .get(&user)
            .map(|e| e.nodes.iter().copied().collect())
            .unwrap_or_default()
    }

    pub fn is_online(&self, user: u64) -> bool {
        self.get(user) != Status::Offline
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_and_clear_single_node() {
        let p = Presence::new();
        assert_eq!(p.get(1), Status::Offline);
        assert!(p.set(1, 10, Status::Online), "offline→online 전이");
        assert!(p.is_online(1));
        assert!(!p.set(1, 10, Status::Online), "이미 online → 변화 없음");
        assert!(p.clear(1, 10), "online→offline 전이");
        assert!(!p.is_online(1));
        assert!(!p.clear(1, 10), "이미 offline → 변화 없음");
    }

    #[test]
    fn nodes_for_tracks_hosting_directory() {
        let p = Presence::new();
        assert!(p.nodes_for(1).is_empty(), "오프라인 유저는 호스팅 노드 없음");
        p.set(1, 10, Status::Online);
        p.set(1, 20, Status::Online);
        let mut nodes = p.nodes_for(1);
        nodes.sort();
        assert_eq!(nodes, vec![10, 20], "두 노드가 이 유저를 호스팅");
        p.clear(1, 10);
        assert_eq!(p.nodes_for(1), vec![20], "노드 하나 빠지면 디렉터리에서 제거");
        p.clear(1, 20);
        assert!(p.nodes_for(1).is_empty(), "전부 빠지면 오프라인=빈 디렉터리");
    }

    #[test]
    fn online_until_all_hosting_nodes_clear() {
        let p = Presence::new();
        assert!(p.set(1, 10, Status::Online));
        assert!(!p.set(1, 20, Status::Online), "둘째 노드 추가는 여전히 online");
        assert!(!p.clear(1, 10), "노드 하나 빠져도 다른 노드에 있어 online 유지");
        assert!(p.is_online(1));
        assert!(p.clear(1, 20), "마지막 노드까지 빠지면 offline");
        assert!(!p.is_online(1));
    }
}
