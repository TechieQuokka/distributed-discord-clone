//! Consistent hashing 링 (개념: ring). Realm을 노드에 배치 (D6).
//! 가상노드(vnode)로 균등 분산. 노드 추가/삭제 시 재배치 최소화 = 일관 해싱의 핵심.
//!
//! 해시는 **결정론적**이어야 한다 — 모든 노드가 같은 링을 계산해야 하므로
//! 랜덤 시드 해셔(std RandomState) 사용 금지. 정수 키엔 splitmix64(강한 avalanche).

use std::collections::BTreeMap;

/// splitmix64 — 순차 정수도 균등 분산(강한 avalanche). 결정론적.
fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn hash_realm(realm_id: u64) -> u64 {
    splitmix64(realm_id)
}

fn vnode_hash(node_id: u64, vnode: u32) -> u64 {
    // node와 vnode를 독립적으로 섞어 결합 → (node,vnode)별 고유·균등.
    splitmix64(node_id ^ splitmix64(vnode as u64))
}

/// 일관 해싱 링. `vnode_hash → node_id` 정렬 맵.
pub struct HashRing {
    vnodes: u32,
    ring: BTreeMap<u64, u64>,
}

impl HashRing {
    /// `vnodes` = 노드당 가상노드 수 (클수록 분산 균등, 보통 100~200).
    pub fn new(vnodes: u32) -> Self {
        assert!(vnodes > 0, "vnodes must be > 0");
        Self { vnodes, ring: BTreeMap::new() }
    }

    pub fn add_node(&mut self, node_id: u64) {
        for v in 0..self.vnodes {
            self.ring.insert(vnode_hash(node_id, v), node_id);
        }
    }

    pub fn remove_node(&mut self, node_id: u64) {
        self.ring.retain(|_, v| *v != node_id);
    }

    /// Realm의 소유 노드 = 해시 이상(>=)인 첫 vnode, 없으면 wrap-around.
    pub fn owner(&self, realm_id: u64) -> Option<u64> {
        if self.ring.is_empty() {
            return None;
        }
        let h = hash_realm(realm_id);
        self.ring
            .range(h..)
            .next()
            .map(|(_, &node)| node)
            .or_else(|| self.ring.values().next().copied())
    }

    pub fn is_empty(&self) -> bool {
        self.ring.is_empty()
    }

    /// 링에 존재하는 고유 노드 수.
    pub fn node_count(&self) -> usize {
        self.ring.values().collect::<std::collections::HashSet<_>>().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn empty_ring_has_no_owner() {
        assert_eq!(HashRing::new(50).owner(123), None);
    }

    #[test]
    fn single_node_owns_everything() {
        let mut r = HashRing::new(50);
        r.add_node(1);
        for realm in 0..200u64 {
            assert_eq!(r.owner(realm), Some(1));
        }
    }

    #[test]
    fn distributes_across_nodes() {
        let mut r = HashRing::new(150);
        for n in [1u64, 2, 3] {
            r.add_node(n);
        }
        assert_eq!(r.node_count(), 3);
        let mut counts: HashMap<u64, u32> = HashMap::new();
        for realm in 0..3000u64 {
            *counts.entry(r.owner(realm).unwrap()).or_default() += 1;
        }
        assert_eq!(counts.len(), 3);
        // 이상적 균등 = 1000. 대략 균등(>500)인지 확인.
        for (_, c) in counts {
            assert!(c > 500, "uneven distribution: {c}");
        }
    }

    #[test]
    fn removal_remaps_only_affected_realms() {
        let mut r = HashRing::new(150);
        for n in [1u64, 2, 3] {
            r.add_node(n);
        }
        let before: Vec<(u64, u64)> = (0..2000u64).map(|x| (x, r.owner(x).unwrap())).collect();

        r.remove_node(3);

        let mut moved = 0;
        for (x, old) in before {
            let new = r.owner(x).unwrap();
            if old == 3 {
                assert_ne!(new, 3, "removed node still owns realm {x}");
                moved += 1;
            } else {
                // 일관 해싱 핵심: 영향 없는 Realm은 소유 노드 유지
                assert_eq!(new, old, "realm {x} moved unnecessarily");
            }
        }
        // node3 소유분만 이동 → 대략 1/3 이하
        assert!(moved > 0 && moved < 1000, "moved={moved}");
    }
}
