//! CRDT 툴킷 — 오프라인 동기화의 순수 merge 엔진 (개념: crdt). D49.
//!
//! 상태 기반 CRDT(CvRDT): 각 복제본이 로컬 상태를 들고 오프라인 편집 → 재연결 시 `merge`(join)로
//! **충돌 없이 수렴**. merge는 **결합·교환·멱등**(semilattice join)이어야 한다 — 이 법칙이 곧
//! "어떤 순서로/몇 번 합쳐도 같은 상태"를 보장(네트워크가 재정렬·중복·재전송해도 안전).
//!
//! IO 없음(P2) → 법칙을 단위 테스트로 고정(DST 친화, D25). 직렬화(wire/DB)는 어댑터가 소유.
//!
//! 제공:
//! - [`LwwRegister`] — last-write-wins 레지스터 (값 1개, (ts,node) 타이브레이크).
//! - [`LwwMap`] — 키별 LWW + 툼스톤 삭제 (유저 동기화 문서의 기반).
//! - [`OrSet`] — observed-remove set (add/remove가 충돌 없이 병합, 쇼케이스).
//! - [`PnCounter`] — 증가/감소 카운터 (per-node).

use std::collections::BTreeMap;

/// (timestamp_ms, node_id) 전순서 — LWW 타이브레이크. 큰 쪽이 이긴다(동시각이면 node_id로 결정).
pub type Dot = (u64, u64);

/// 유저 동기화 문서의 한 엔트리 — wire/DB ↔ [`LwwMap`] 변환 단위 (D49). value=None은 툼스톤.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CrdtEntry {
    pub key: String,
    pub value: Option<String>,
    pub ts_ms: u64,
    pub node: u64,
}

impl LwwMap {
    /// 엔트리 목록으로 LwwMap 구성 (DB/wire 적재).
    pub fn from_entries(entries: impl IntoIterator<Item = CrdtEntry>) -> Self {
        let mut m = Self::new();
        for e in entries {
            m.apply_raw(&e.key, e.value, e.ts_ms, e.node);
        }
        m
    }
    /// 엔트리 목록으로 추출 (툼스톤 포함).
    pub fn to_entries(&self) -> Vec<CrdtEntry> {
        self.raw_entries()
            .into_iter()
            .map(|(key, value, ts_ms, node)| CrdtEntry { key, value, ts_ms, node })
            .collect()
    }
}

/// Last-Write-Wins 레지스터. 값 1개를 들고, merge는 더 큰 `dot`을 채택.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LwwRegister<V: Clone + Eq> {
    pub value: V,
    pub dot: Dot,
}

impl<V: Clone + Eq> LwwRegister<V> {
    pub fn new(value: V, ts_ms: u64, node: u64) -> Self {
        Self { value, dot: (ts_ms, node) }
    }
    /// 더 큰 dot이 이김(결정론적 타이브레이크). 같은 dot이면 그대로(멱등).
    pub fn merge(&mut self, other: &Self) {
        if other.dot > self.dot {
            self.value = other.value.clone();
            self.dot = other.dot;
        }
    }
    pub fn merged(mut self, other: &Self) -> Self {
        self.merge(other);
        self
    }
}

/// LWW-Map: 키→(값, dot). 삭제는 **툼스톤**(값 없음 + dot) — 삭제도 LWW라 "삭제 후 더 늦은 쓰기"가 부활.
/// 유저 동기화 문서(드래프트·설정)의 기반. 오프라인 두 기기가 같은 키를 바꿔도 더 늦은 쓰기로 수렴.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LwwMap {
    /// key → (value: Some=존재 / None=툼스톤, dot)
    entries: BTreeMap<String, (Option<String>, Dot)>,
}

impl LwwMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// 키 설정(쓰기). 기존보다 dot이 크면 채택.
    pub fn set(&mut self, key: &str, value: String, ts_ms: u64, node: u64) {
        self.apply(key, Some(value), (ts_ms, node));
    }

    /// 키 삭제(툼스톤). 기존보다 dot이 크면 채택(LWW 삭제).
    pub fn remove(&mut self, key: &str, ts_ms: u64, node: u64) {
        self.apply(key, None, (ts_ms, node));
    }

    fn apply(&mut self, key: &str, value: Option<String>, dot: Dot) {
        match self.entries.get(key) {
            Some((_, cur)) if *cur >= dot => {} // 기존이 더 최신(또는 동일) → 무시(멱등).
            _ => {
                self.entries.insert(key.to_string(), (value, dot));
            }
        }
    }

    /// 현재 살아있는 키→값 (툼스톤 제외).
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key).and_then(|(v, _)| v.as_deref())
    }

    /// 살아있는 항목 목록 (키 정렬). 툼스톤 제외.
    pub fn live(&self) -> Vec<(&str, &str)> {
        self.entries
            .iter()
            .filter_map(|(k, (v, _))| v.as_deref().map(|val| (k.as_str(), val)))
            .collect()
    }

    /// 모든 항목(툼스톤 포함) — 직렬화/동기화용. (key, value?, ts, node).
    pub fn raw_entries(&self) -> Vec<(String, Option<String>, u64, u64)> {
        self.entries
            .iter()
            .map(|(k, (v, (ts, n)))| (k.clone(), v.clone(), *ts, *n))
            .collect()
    }

    /// 어댑터가 저장분을 적재할 때 사용 (dot 그대로 주입).
    pub fn apply_raw(&mut self, key: &str, value: Option<String>, ts_ms: u64, node: u64) {
        self.apply(key, value, (ts_ms, node));
    }

    /// 두 복제본 병합(join). 키별 LWW. **결합·교환·멱등**.
    pub fn merge(&mut self, other: &LwwMap) {
        for (k, (v, dot)) in &other.entries {
            self.apply(k, v.clone(), *dot);
        }
    }
    pub fn merged(mut self, other: &LwwMap) -> Self {
        self.merge(other);
        self
    }
}

/// Observed-Remove Set: 원소를 고유 태그(dot)와 함께 추가 → 삭제는 **관측된 태그**만 제거.
/// "add가 concurrent remove를 이긴다"(add-wins) — 두 기기가 같은 원소를 동시에 add/remove해도 충돌 없음.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OrSet<T: Ord + Clone> {
    /// element → 관측된 add 태그 집합 (살아있는 태그가 하나라도 있으면 원소 존재).
    adds: BTreeMap<T, std::collections::BTreeSet<Dot>>,
    /// 제거된(관측된) 태그 — 같은 태그의 재유입 방지.
    removed: std::collections::BTreeSet<Dot>,
}

impl<T: Ord + Clone> OrSet<T> {
    pub fn new() -> Self {
        Self { adds: BTreeMap::new(), removed: std::collections::BTreeSet::new() }
    }

    /// 고유 태그로 원소 추가.
    pub fn add(&mut self, value: T, ts_ms: u64, node: u64) {
        self.adds.entry(value).or_default().insert((ts_ms, node));
    }

    /// 원소의 **현재 관측된** add 태그를 모두 제거(observed-remove). 이후 들어온 add는 살아남음.
    pub fn remove(&mut self, value: &T) {
        if let Some(tags) = self.adds.get(value) {
            for t in tags {
                self.removed.insert(*t);
            }
        }
    }

    /// 살아있는 원소 = add 태그 중 removed에 없는 게 하나라도 있는 것.
    pub fn contains(&self, value: &T) -> bool {
        self.adds.get(value).is_some_and(|tags| tags.iter().any(|t| !self.removed.contains(t)))
    }

    pub fn elements(&self) -> Vec<T> {
        self.adds
            .iter()
            .filter(|(_, tags)| tags.iter().any(|t| !self.removed.contains(t)))
            .map(|(v, _)| v.clone())
            .collect()
    }

    /// join: add 태그 합집합 + removed 합집합. 결합·교환·멱등.
    pub fn merge(&mut self, other: &OrSet<T>) {
        for (v, tags) in &other.adds {
            self.adds.entry(v.clone()).or_default().extend(tags.iter().copied());
        }
        self.removed.extend(other.removed.iter().copied());
    }
}

/// PN-Counter: per-node 증가(p)/감소(n) 합. merge는 노드별 max(단조). 값 = Σp - Σn.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PnCounter {
    p: BTreeMap<u64, u64>,
    n: BTreeMap<u64, u64>,
}

impl PnCounter {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn increment(&mut self, node: u64, by: u64) {
        *self.p.entry(node).or_default() += by;
    }
    pub fn decrement(&mut self, node: u64, by: u64) {
        *self.n.entry(node).or_default() += by;
    }
    pub fn value(&self) -> i64 {
        let sum = |m: &BTreeMap<u64, u64>| m.values().map(|&v| v as i64).sum::<i64>();
        sum(&self.p) - sum(&self.n)
    }
    /// 노드별 max(단조 증가라 max가 join). 결합·교환·멱등.
    pub fn merge(&mut self, other: &PnCounter) {
        for (k, v) in &other.p {
            let e = self.p.entry(*k).or_default();
            *e = (*e).max(*v);
        }
        for (k, v) in &other.n {
            let e = self.n.entry(*k).or_default();
            *e = (*e).max(*v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- LwwRegister ----
    #[test]
    fn lww_register_higher_dot_wins() {
        let mut a = LwwRegister::new("a", 10, 1);
        a.merge(&LwwRegister::new("b", 20, 1)); // 더 늦음 → b
        assert_eq!(a.value, "b");
        a.merge(&LwwRegister::new("c", 15, 1)); // 더 이름 → 무시
        assert_eq!(a.value, "b");
    }
    #[test]
    fn lww_register_tiebreak_by_node() {
        let mut a = LwwRegister::new("a", 10, 1);
        a.merge(&LwwRegister::new("b", 10, 2)); // 동시각, node 2 > 1 → b
        assert_eq!(a.value, "b");
    }

    // ---- LwwMap: CRDT 법칙 ----
    fn sample_map(node: u64) -> LwwMap {
        let mut m = LwwMap::new();
        m.set("k1", format!("v{node}"), 100 + node, node);
        m
    }

    #[test]
    fn lwwmap_merge_is_idempotent_commutative_associative() {
        let a = sample_map(1);
        let b = sample_map(2);
        let mut c = LwwMap::new();
        c.set("k2", "z".into(), 50, 3);

        // 멱등: a∨a = a
        assert_eq!(a.clone().merged(&a), a);
        // 교환: a∨b = b∨a
        assert_eq!(a.clone().merged(&b), b.clone().merged(&a));
        // 결합: (a∨b)∨c = a∨(b∨c)
        let left = a.clone().merged(&b).merged(&c);
        let right = a.clone().merged(&b.clone().merged(&c));
        assert_eq!(left, right);
    }

    #[test]
    fn lwwmap_offline_two_devices_converge() {
        // 기기1: 오프라인에서 k="phone" (ts 200), 기기2: k="laptop" (ts 210) — 둘 다 같은 키 편집.
        let mut d1 = LwwMap::new();
        d1.set("draft", "phone".into(), 200, 1);
        let mut d2 = LwwMap::new();
        d2.set("draft", "laptop".into(), 210, 2);
        // 양방향 동기화 후 둘 다 같은 상태(더 늦은 laptop).
        let s1 = d1.clone().merged(&d2);
        let s2 = d2.clone().merged(&d1);
        assert_eq!(s1, s2);
        assert_eq!(s1.get("draft"), Some("laptop"));
    }

    #[test]
    fn lwwmap_tombstone_delete_and_resurrect() {
        let mut m = LwwMap::new();
        m.set("k", "v".into(), 10, 1);
        m.remove("k", 20, 1); // 삭제(툼스톤)
        assert_eq!(m.get("k"), None);
        m.set("k", "again".into(), 30, 1); // 더 늦은 쓰기 → 부활
        assert_eq!(m.get("k"), Some("again"));
        // 늦은 삭제가 이른 쓰기를 못 되살림(멱등): 같은 merge 반복해도 안정.
        let snapshot = m.clone();
        assert_eq!(m.clone().merged(&snapshot), m);
    }

    // ---- OrSet: add-wins ----
    #[test]
    fn orset_add_wins_over_concurrent_remove() {
        // r1: x 추가 후 제거. r2: 같은 x를 (다른 태그로) 추가. merge → x 살아있음(add-wins).
        let mut r1 = OrSet::new();
        r1.add("x", 10, 1);
        r1.remove(&"x");
        let mut r2 = OrSet::new();
        r2.add("x", 11, 2);
        r1.merge(&r2);
        assert!(r1.contains(&"x"), "concurrent add가 remove를 이긴다");
        assert_eq!(r1.elements(), vec!["x"]);
    }
    #[test]
    fn orset_merge_commutative() {
        let mut a = OrSet::new();
        a.add("a", 1, 1);
        let mut b = OrSet::new();
        b.add("b", 1, 2);
        b.remove(&"b");
        let ab = {
            let mut x = a.clone();
            x.merge(&b);
            x.elements()
        };
        let ba = {
            let mut x = b.clone();
            x.merge(&a);
            x.elements()
        };
        assert_eq!(ab, ba);
        assert_eq!(ab, vec!["a"]);
    }

    // ---- PnCounter ----
    #[test]
    fn pncounter_converges_with_max_join() {
        let mut a = PnCounter::new();
        a.increment(1, 5);
        a.decrement(1, 2);
        let mut b = PnCounter::new();
        b.increment(2, 3);
        a.merge(&b);
        b.merge(&a);
        assert_eq!(a.value(), b.value());
        assert_eq!(a.value(), 5 - 2 + 3);
        // 멱등: 재병합해도 불변.
        let snap = a.clone();
        a.merge(&snap);
        assert_eq!(a.value(), 6);
    }
}
