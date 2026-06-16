//! SWIM 멤버십 (개념: swim). 동적 노드 합류/이탈 + 의심(suspicion) 기반 장애감지 (D45).
//!
//! SWIM = Scalable Weakly-consistent Infection-style process group Membership.
//! Phase 2~4의 정적 config + 단순 PING/PONG(D23)을 일반화한다:
//!  1. **3상태 + incarnation**: Alive/Suspect/Dead. 충돌 해소 = 높은 incarnation 우선,
//!     같으면 Dead>Suspect>Alive. 자기에 대한 Suspect/Dead를 보면 incarnation을 올려 Alive 반박(refute).
//!  2. **direct + indirect 탐침**: 주기마다 1명 ping → 타임아웃 시 k명에게 ping-req 위임(오탐 감소) →
//!     직접·간접 모두 실패해야 Suspect → suspicion 타임아웃 후 Dead.
//!  3. **감염형 전파**: 멤버 델타를 ping/ack/gossip에 피기백, 각 업데이트는 유한 횟수만 재전파.
//!
//! **경계 (P2/P5)**: 이 모듈은 **순수 상태 + now_ms/rng 주입**. 실제 송신·링 변형·dial은
//! `SwimAction`으로 돌려주고 드라이버(`run_swim`)가 transport/Router로 수행 → DST(D25) 결정론 재현.

use std::collections::HashMap;
use std::sync::Mutex;

use protocol::{NodeMessage, SwimMember};

/// 멤버 상태 (wire u8: 0=Alive 1=Suspect 2=Dead).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MemberState {
    Alive,
    Suspect,
    Dead,
}

impl MemberState {
    pub fn as_u8(self) -> u8 {
        match self {
            MemberState::Alive => 0,
            MemberState::Suspect => 1,
            MemberState::Dead => 2,
        }
    }
    pub fn from_u8(b: u8) -> MemberState {
        match b {
            1 => MemberState::Suspect,
            2 => MemberState::Dead,
            _ => MemberState::Alive,
        }
    }
    /// 충돌 해소 우선순위 (같은 incarnation에서 Dead가 가장 강함).
    fn rank(self) -> u8 {
        self.as_u8()
    }
}

/// SWIM 튜닝 파라미터. 기본은 로컬 study용(빠른 수렴). DST는 작은 값으로 결정론 재현.
#[derive(Clone, Copy, Debug)]
pub struct SwimConfig {
    /// direct ping 후 indirect로 넘어가기까지 대기(ms).
    pub ping_timeout_ms: u64,
    /// 한 탐침 주기 총 길이(ms) — 이 안에 ack 없으면 Suspect.
    pub probe_period_ms: u64,
    /// Suspect → Dead 전이까지 대기(ms).
    pub suspicion_timeout_ms: u64,
    /// 간접 탐침 대리 노드 수 k.
    pub indirect_k: usize,
    /// gossip 1회에 보낼 대상 수.
    pub gossip_fanout: usize,
    /// 한 멤버 델타를 재전파할 최대 횟수(≈ λ·logN). 클수록 확실히 퍼지나 트래픽↑.
    pub dissemination_count: u32,
    /// 피기백/gossip 배치 최대 멤버 수.
    pub max_piggyback: usize,
    /// 주기적 full-snapshot anti-entropy 간격(tick 수). bounded 전파 누락 대비 수렴 보강. 0이면 비활성.
    pub anti_entropy_ticks: u64,
}

impl Default for SwimConfig {
    fn default() -> Self {
        Self {
            ping_timeout_ms: 400,
            probe_period_ms: 1_000,
            suspicion_timeout_ms: 3_000,
            indirect_k: 2,
            gossip_fanout: 2,
            dissemination_count: 6,
            max_piggyback: 8,
            anti_entropy_ticks: 10,
        }
    }
}

/// 드라이버가 실행할 부수효과 — 송신/링 변형. (IO 격리, P2)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwimAction {
    /// 이 노드로 메시지 전송.
    Send { to: u64, msg: NodeMessage },
    /// 링에 노드 추가(Alive) — 신규면 드라이버가 dial + presence anti-entropy(D46) 트리거.
    RingAdd { node: u64, addr: String },
    /// 링에서 노드 제거(Dead 확정).
    RingRemove { node: u64 },
    /// 소유권 부여 제외(Suspect) — 드라이버가 membership.mark_down.
    Suspect { node: u64 },
}

/// 결정론 PRNG (splitmix64) — 탐침 대상/대리 선택. 시드 동일 → 동일 선택열 (D25).
pub struct SwimRng(u64);
impl SwimRng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

#[derive(Clone)]
struct Member {
    addr: String,
    incarnation: u64,
    state: MemberState,
    /// 현재 상태로 진입한 시각(ms) — suspicion/GC 타임아웃 기준.
    since_ms: u64,
    /// 남은 재전파 횟수(감염형 전파 bound).
    dissem_left: u32,
}

impl Member {
    fn as_wire(&self, node_id: u64) -> SwimMember {
        SwimMember {
            node_id,
            addr: self.addr.clone(),
            incarnation: self.incarnation,
            state: self.state.as_u8(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ProbeStage {
    Direct,
    Indirect,
}

struct Probe {
    node: u64,
    started_ms: u64,
    stage: ProbeStage,
    /// indirect 단계에서 ping-req를 위임한 대리 노드들 — 이들의 ack = 타깃 alive.
    relays: Vec<u64>,
}

struct Inner {
    local: u64,
    local_addr: String,
    local_incarnation: u64,
    /// 자기 Alive를 알리기 위한 잔여 전파 횟수(refute/announce).
    self_dissem_left: u32,
    members: HashMap<u64, Member>,
    /// node → 마지막으로 그 노드의 직접 트래픽을 받은 시각(ms). 탐침 성공 판정 보조.
    last_seen_ms: HashMap<u64, u64>,
    /// 진행 중 탐침(주기당 1개).
    probe: Option<Probe>,
    /// 대리(relay)로서 타깃에 보낸 ping의 seq → 원 요청자. 타깃 ack 시 요청자에 중계.
    relay_pending: HashMap<u64, u64>,
    seq: u64,
    rng: SwimRng,
    cfg: SwimConfig,
    /// 합류 완료 여부(H1) — seed로부터 SwimGossip/Ack 수신 시 set. dynamic 노드는 이게 true 될 때까지 join 재전송.
    bootstrapped: bool,
    /// 라운드로빈 probe 순서(H2) + 현재 인덱스. 소진 시 재셔플 → 한 라운드에 각 멤버 1회 탐지.
    probe_order: Vec<u64>,
    probe_idx: usize,
    /// step 호출 횟수(H3 anti-entropy 주기 기준).
    tick_count: u64,
}

/// SWIM 멤버십 뷰. server가 `Arc`로 소유해 드라이버 + inbound 루프가 공유.
pub struct Swim {
    inner: Mutex<Inner>,
}

impl Swim {
    /// `local_addr` = 이 노드의 advertise 주소(피어가 dial할 "host:port"). `seed_rng` = 결정론 시드.
    pub fn new(local: u64, local_addr: impl Into<String>, cfg: SwimConfig, seed_rng: u64) -> Self {
        Self {
            inner: Mutex::new(Inner {
                local,
                local_addr: local_addr.into(),
                local_incarnation: 0,
                self_dissem_left: cfg.dissemination_count,
                members: HashMap::new(),
                last_seen_ms: HashMap::new(),
                probe: None,
                relay_pending: HashMap::new(),
                seq: 0,
                rng: SwimRng::new(seed_rng),
                cfg,
                bootstrapped: false,
                probe_order: Vec::new(),
                probe_idx: 0,
                tick_count: 0,
            }),
        }
    }

    /// 시작 시 알려진 피어(정적 config seed 또는 전체목록) 주입 — Alive로 시드.
    /// 링 추가는 드라이버/서버가 별도로 (또는 반환 액션으로) 처리.
    pub fn seed_member(&self, node: u64, addr: impl Into<String>) {
        let mut g = self.inner.lock().unwrap();
        let cfg = g.cfg;
        if node == g.local {
            return;
        }
        g.members.entry(node).or_insert(Member {
            addr: addr.into(),
            incarnation: 0,
            state: MemberState::Alive,
            since_ms: 0,
            dissem_left: cfg.dissemination_count,
        });
    }

    /// 현재 살아있다고(또는 의심) 보는 멤버의 (node, addr) 목록 — 서버가 시작 시 링/ dial 구성에 사용.
    pub fn live_members(&self) -> Vec<(u64, String)> {
        let g = self.inner.lock().unwrap();
        let mut v: Vec<(u64, String)> = g
            .members
            .iter()
            .filter(|(_, m)| m.state != MemberState::Dead)
            .map(|(id, m)| (*id, m.addr.clone()))
            .collect();
        v.sort_by_key(|(id, _)| *id);
        v
    }

    pub fn state_of(&self, node: u64) -> Option<MemberState> {
        self.inner.lock().unwrap().members.get(&node).map(|m| m.state)
    }

    pub fn local_incarnation(&self) -> u64 {
        self.inner.lock().unwrap().local_incarnation
    }

    /// 합류 완료 여부 (H1) — seed로부터 응답(SwimGossip/Ack)을 받았는가.
    pub fn bootstrapped(&self) -> bool {
        self.inner.lock().unwrap().bootstrapped
    }

    /// 전체 멤버 테이블 스냅샷(자기 포함) — join 응답/디버깅용.
    pub fn snapshot(&self) -> Vec<SwimMember> {
        self.inner.lock().unwrap().full_snapshot()
    }

    // ─── 핵심: 인바운드 메시지 합병 + 주기 step ─────────────────────────────

    /// 인바운드 SWIM 메시지 처리 → 부수효과. server inbound 루프가 호출.
    pub fn handle(&self, from: u64, msg: &NodeMessage, now_ms: u64) -> Vec<SwimAction> {
        let mut g = self.inner.lock().unwrap();
        g.last_seen_ms.insert(from, now_ms);
        // from 자체가 살아있다는 증거 → 진행 중 탐침이 from을 향했으면 성공.
        g.resolve_probe_on_traffic(from);

        let mut actions = Vec::new();
        match msg {
            NodeMessage::SwimJoin { addr, incarnation } => {
                // 나는 seed(introducer): 합류자를 Alive로 합병 + 전체 테이블 회신.
                let upd = SwimMember { node_id: from, addr: addr.clone(), incarnation: *incarnation, state: 0 };
                g.merge_one(&upd, now_ms, &mut actions);
                let snap = g.full_snapshot();
                actions.push(SwimAction::Send { to: from, msg: NodeMessage::SwimGossip { updates: snap } });
            }
            NodeMessage::SwimPing { seq, updates } => {
                g.merge_many(updates, now_ms, &mut actions);
                let piggy = g.hot_updates(now_ms);
                actions.push(SwimAction::Send { to: from, msg: NodeMessage::SwimAck { seq: *seq, updates: piggy } });
            }
            NodeMessage::SwimAck { seq, updates } => {
                g.merge_many(updates, now_ms, &mut actions);
                g.bootstrapped = true; // seed/peer 응답 = 합류 확인(H1).
                // 내가 대리(relay)로 보냈던 ping의 응답이면 → 원 요청자에게 중계(타깃 alive 증거).
                if let Some(requester) = g.relay_pending.remove(seq) {
                    let piggy = g.hot_updates(now_ms);
                    actions.push(SwimAction::Send {
                        to: requester,
                        msg: NodeMessage::SwimAck { seq: *seq, updates: piggy },
                    });
                }
            }
            NodeMessage::SwimPingReq { seq: _, target, target_addr, updates } => {
                g.merge_many(updates, now_ms, &mut actions);
                // 대리 탐침: 타깃을 대신 ping하고, 그 ack를 요청자에게 중계하도록 기록.
                let s2 = g.next_seq();
                g.relay_pending.insert(s2, from);
                // 타깃을 모르면 주소만 들고 시도(전송 실패는 fire-and-forget).
                let piggy = g.hot_updates(now_ms);
                let _ = target_addr; // 주소는 드라이버가 라우팅 불필요(이미 연결됐을 때만 도달)
                actions.push(SwimAction::Send {
                    to: *target,
                    msg: NodeMessage::SwimPing { seq: s2, updates: piggy },
                });
            }
            NodeMessage::SwimGossip { updates } => {
                g.merge_many(updates, now_ms, &mut actions);
                g.bootstrapped = true; // join 응답(전체 테이블) 수신 = 합류 확인(H1).
            }
            _ => {}
        }
        actions
    }

    /// 주기 탐침/타임아웃 진행 → 부수효과. 드라이버가 interval마다 호출.
    pub fn step(&self, now_ms: u64) -> Vec<SwimAction> {
        let mut g = self.inner.lock().unwrap();
        let mut actions = Vec::new();

        // 1) suspicion 타임아웃 → Dead.
        g.sweep_suspects(now_ms, &mut actions);
        // 2) 진행 중 탐침 단계 진행 (direct→indirect→suspect).
        g.advance_probe(now_ms, &mut actions);
        // 3) 탐침 없으면 새 대상 선정 → ping.
        g.start_probe(now_ms, &mut actions);
        // 4) hot 멤버 델타를 gossip으로 추가 확산.
        let piggy = g.hot_updates(now_ms);
        if !piggy.is_empty() {
            for to in g.gossip_targets() {
                actions.push(SwimAction::Send { to, msg: NodeMessage::SwimGossip { updates: piggy.clone() } });
            }
        }
        // 5) 주기적 full-snapshot anti-entropy (H3).
        g.anti_entropy(&mut actions);
        // 6) Dead 멤버 GC (충분히 전파된 뒤).
        g.gc_dead(now_ms);
        actions
    }

    /// 이 노드가 seed에게 보낼 합류 요청 메시지(드라이버가 startup에 1회 send).
    pub fn join_message(&self) -> NodeMessage {
        let g = self.inner.lock().unwrap();
        NodeMessage::SwimJoin { addr: g.local_addr.clone(), incarnation: g.local_incarnation }
    }
}

impl Inner {
    fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    /// from의 직접 트래픽 도착 → 진행 중 탐침이 from(직접) 또는 relay(간접)면 성공 처리.
    fn resolve_probe_on_traffic(&mut self, from: u64) {
        let done = match &self.probe {
            Some(p) if p.node == from => true,
            Some(p) if p.stage == ProbeStage::Indirect && p.relays.contains(&from) => true,
            _ => false,
        };
        if done {
            self.probe = None;
        }
    }

    /// 멤버 델타 1건 합병 (충돌 해소 + 부수효과). self 대상이면 refute.
    fn merge_one(&mut self, u: &SwimMember, now: u64, actions: &mut Vec<SwimAction>) {
        let new_state = MemberState::from_u8(u.state);
        if u.node_id == self.local {
            // 나에 대한 Suspect/Dead → incarnation 올려 Alive 반박.
            if new_state != MemberState::Alive && u.incarnation >= self.local_incarnation {
                self.local_incarnation = u.incarnation + 1;
                self.self_dissem_left = self.cfg.dissemination_count;
            }
            return;
        }
        match self.members.get_mut(&u.node_id) {
            None => {
                if new_state == MemberState::Dead {
                    return; // 모르던 노드의 Dead는 무시(부활 방지).
                }
                self.members.insert(
                    u.node_id,
                    Member {
                        addr: u.addr.clone(),
                        incarnation: u.incarnation,
                        state: new_state,
                        since_ms: now,
                        dissem_left: self.cfg.dissemination_count,
                    },
                );
                actions.push(SwimAction::RingAdd { node: u.node_id, addr: u.addr.clone() });
                if new_state == MemberState::Suspect {
                    actions.push(SwimAction::Suspect { node: u.node_id });
                }
            }
            Some(m) => {
                let wins = if u.incarnation != m.incarnation {
                    u.incarnation > m.incarnation
                } else {
                    new_state.rank() > m.state.rank()
                };
                if !wins {
                    return;
                }
                let was = m.state;
                m.incarnation = u.incarnation;
                m.state = new_state;
                m.since_ms = now;
                m.dissem_left = self.cfg.dissemination_count;
                if !u.addr.is_empty() {
                    m.addr = u.addr.clone();
                }
                match new_state {
                    MemberState::Alive => {
                        if was != MemberState::Alive {
                            actions.push(SwimAction::RingAdd { node: u.node_id, addr: m.addr.clone() });
                        }
                    }
                    MemberState::Suspect => actions.push(SwimAction::Suspect { node: u.node_id }),
                    MemberState::Dead => actions.push(SwimAction::RingRemove { node: u.node_id }),
                }
            }
        }
    }

    fn merge_many(&mut self, updates: &[SwimMember], now: u64, actions: &mut Vec<SwimAction>) {
        for u in updates {
            self.merge_one(u, now, actions);
        }
    }

    /// Suspect가 suspicion_timeout 초과 → Dead.
    fn sweep_suspects(&mut self, now: u64, actions: &mut Vec<SwimAction>) {
        let timeout = self.cfg.suspicion_timeout_ms;
        let fanout = self.cfg.dissemination_count;
        let mut dead = Vec::new();
        for (id, m) in self.members.iter_mut() {
            if m.state == MemberState::Suspect && now.saturating_sub(m.since_ms) >= timeout {
                m.state = MemberState::Dead;
                m.since_ms = now;
                m.dissem_left = fanout;
                dead.push(*id);
            }
        }
        for id in dead {
            actions.push(SwimAction::RingRemove { node: id });
        }
    }

    /// 진행 중 탐침 단계 진행.
    fn advance_probe(&mut self, now: u64, actions: &mut Vec<SwimAction>) {
        let Some(p) = self.probe.as_ref() else { return };
        let (node, started, stage) = (p.node, p.started_ms, p.stage);
        // 직접 트래픽으로 이미 살아있음이 확인됐으면 성공 (탐침 시작 이후 도착분만 — strict).
        if self.last_seen_ms.get(&node).copied().unwrap_or(0) > started {
            self.probe = None;
            return;
        }
        match stage {
            ProbeStage::Direct => {
                if now.saturating_sub(started) >= self.cfg.ping_timeout_ms {
                    // 간접 탐침: k명에게 ping-req 위임.
                    let relays = self.k_random_alive(node, self.cfg.indirect_k);
                    let target_addr =
                        self.members.get(&node).map(|m| m.addr.clone()).unwrap_or_default();
                    let piggy = self.hot_updates(now);
                    for r in &relays {
                        let seq = self.next_seq();
                        actions.push(SwimAction::Send {
                            to: *r,
                            msg: NodeMessage::SwimPingReq {
                                seq,
                                target: node,
                                target_addr: target_addr.clone(),
                                updates: piggy.clone(),
                            },
                        });
                    }
                    if let Some(p) = self.probe.as_mut() {
                        p.stage = ProbeStage::Indirect;
                        p.relays = relays;
                    }
                }
            }
            ProbeStage::Indirect => {
                if now.saturating_sub(started) >= self.cfg.probe_period_ms {
                    // 직접·간접 모두 실패 → Suspect (Alive였던 경우만).
                    if let Some(m) = self.members.get_mut(&node) {
                        if m.state == MemberState::Alive {
                            m.state = MemberState::Suspect;
                            m.since_ms = now;
                            m.dissem_left = self.cfg.dissemination_count;
                            actions.push(SwimAction::Suspect { node });
                        }
                    }
                    self.probe = None;
                }
            }
        }
    }

    /// 탐침 없으면 새 대상(라운드로빈, H2) 선정 → ping.
    fn start_probe(&mut self, now: u64, actions: &mut Vec<SwimAction>) {
        if self.probe.is_some() {
            return;
        }
        let Some(node) = self.next_probe_target() else { return };
        let seq = self.next_seq();
        let piggy = self.hot_updates(now);
        actions.push(SwimAction::Send { to: node, msg: NodeMessage::SwimPing { seq, updates: piggy } });
        self.probe = Some(Probe { node, started_ms: now, stage: ProbeStage::Direct, relays: Vec::new() });
    }

    /// 라운드로빈 probe 대상 (H2): 셔플된 순서로 각 멤버를 한 번씩, 소진 시 재셔플 →
    /// 한 라운드에 모든 멤버를 정확히 1회 탐지(임의 선택의 탐지시간 변동을 없앰, SWIM 정석).
    fn next_probe_target(&mut self) -> Option<u64> {
        loop {
            if self.probe_idx >= self.probe_order.len() {
                let mut order = self.probe_candidates(); // 현재 non-Dead 멤버(정렬).
                for i in (1..order.len()).rev() {
                    let j = (self.rng.next() % (i as u64 + 1)) as usize;
                    order.swap(i, j);
                }
                if order.is_empty() {
                    self.probe_order.clear();
                    self.probe_idx = 0;
                    return None;
                }
                self.probe_order = order;
                self.probe_idx = 0;
            }
            let cand = self.probe_order[self.probe_idx];
            self.probe_idx += 1;
            // 라운드 도중 죽거나 사라진 멤버는 건너뜀.
            if self.members.get(&cand).map(|m| m.state != MemberState::Dead).unwrap_or(false) {
                return Some(cand);
            }
        }
    }

    /// 주기적 full-snapshot anti-entropy (H3): bounded 전파(dissem_left) 누락 대비 —
    /// 일정 tick마다 전체 멤버 테이블을 임의 멤버 1명에게 push해 view를 수렴시킨다.
    fn anti_entropy(&mut self, actions: &mut Vec<SwimAction>) {
        self.tick_count += 1;
        let n = self.cfg.anti_entropy_ticks;
        if n == 0 || self.tick_count % n != 0 {
            return;
        }
        let candidates = self.probe_candidates();
        if candidates.is_empty() {
            return;
        }
        let target = candidates[(self.rng.next() % candidates.len() as u64) as usize];
        let snap = self.full_snapshot();
        actions.push(SwimAction::Send { to: target, msg: NodeMessage::SwimGossip { updates: snap } });
    }

    /// 전체 멤버 테이블(자기 포함) — anti-entropy/join 응답용.
    fn full_snapshot(&self) -> Vec<SwimMember> {
        let mut v = vec![SwimMember {
            node_id: self.local,
            addr: self.local_addr.clone(),
            incarnation: self.local_incarnation,
            state: MemberState::Alive.as_u8(),
        }];
        let mut ms: Vec<(&u64, &Member)> = self.members.iter().collect();
        ms.sort_by_key(|(id, _)| **id);
        for (id, m) in ms {
            v.push(m.as_wire(*id));
        }
        v
    }

    /// 탐침 후보 = Alive/Suspect 멤버 (정렬, 결정론).
    fn probe_candidates(&self) -> Vec<u64> {
        let mut v: Vec<u64> = self
            .members
            .iter()
            .filter(|(_, m)| m.state != MemberState::Dead)
            .map(|(id, _)| *id)
            .collect();
        v.sort_unstable();
        v
    }

    /// 타깃 제외 k명의 Alive 대리 (rng 셔플 후 take).
    fn k_random_alive(&mut self, exclude: u64, k: usize) -> Vec<u64> {
        let mut pool: Vec<u64> = self
            .members
            .iter()
            .filter(|(id, m)| **id != exclude && m.state == MemberState::Alive)
            .map(|(id, _)| *id)
            .collect();
        pool.sort_unstable();
        // Fisher-Yates (결정론 rng).
        for i in (1..pool.len()).rev() {
            let j = (self.rng.next() % (i as u64 + 1)) as usize;
            pool.swap(i, j);
        }
        pool.truncate(k);
        pool
    }

    /// gossip 확산 대상 (Alive 멤버 중 fanout명, 정렬 후 rng).
    fn gossip_targets(&mut self) -> Vec<u64> {
        let fanout = self.cfg.gossip_fanout;
        let mut pool: Vec<u64> = self
            .members
            .iter()
            .filter(|(_, m)| m.state == MemberState::Alive)
            .map(|(id, _)| *id)
            .collect();
        pool.sort_unstable();
        for i in (1..pool.len()).rev() {
            let j = (self.rng.next() % (i as u64 + 1)) as usize;
            pool.swap(i, j);
        }
        pool.truncate(fanout);
        pool
    }

    /// 재전파 잔여(dissem_left>0)인 멤버 델타 배치 — 자기 announce 포함. 포함분은 카운터 감소.
    fn hot_updates(&mut self, _now: u64) -> Vec<SwimMember> {
        let max = self.cfg.max_piggyback;
        let mut out = Vec::new();
        if self.self_dissem_left > 0 {
            self.self_dissem_left -= 1;
            out.push(SwimMember {
                node_id: self.local,
                addr: self.local_addr.clone(),
                incarnation: self.local_incarnation,
                state: MemberState::Alive.as_u8(),
            });
        }
        let mut ids: Vec<u64> = self
            .members
            .iter()
            .filter(|(_, m)| m.dissem_left > 0)
            .map(|(id, _)| *id)
            .collect();
        ids.sort_unstable();
        for id in ids {
            if out.len() >= max {
                break;
            }
            let m = self.members.get_mut(&id).unwrap();
            m.dissem_left -= 1;
            out.push(m.as_wire(id));
        }
        out
    }

    /// Dead 멤버를 충분히 전파한 뒤 테이블에서 제거(GC).
    fn gc_dead(&mut self, now: u64) {
        let timeout = self.cfg.suspicion_timeout_ms * 2;
        self.members.retain(|_, m| {
            !(m.state == MemberState::Dead && m.dissem_left == 0 && now.saturating_sub(m.since_ms) >= timeout)
        });
    }
}

// ─── 드라이버 (IO 수행) ────────────────────────────────────────────────────

use std::sync::Arc;
use std::time::Duration;

use transport::NodeTransport;

use crate::clock::Clock;
use crate::router::Router;

/// 신규 노드가 Alive로 올라올 때 드라이버가 호출하는 훅 — server가 (1) 런타임 dial,
/// (2) presence anti-entropy 스냅샷 push(D46)를 수행하도록 위임 (node 코어는 transport 구체/presence 무지, P2).
pub type MemberUpHook = Arc<dyn Fn(u64, String) + Send + Sync>;

/// `SwimAction` 목록을 실제 IO로 실행 (드라이버 step + server inbound handle 공용).
pub async fn apply_swim_actions<T: NodeTransport>(
    router: &Router<T>,
    on_member_up: &MemberUpHook,
    now_ms: u64,
    actions: Vec<SwimAction>,
) {
    for a in actions {
        match a {
            SwimAction::Send { to, msg } => router.send_to(to, msg).await,
            SwimAction::RingAdd { node, addr } => {
                router.add_ring_node(node);
                router.membership().record_seen(node, now_ms); // down에서 해제(소유권 복귀)
                (on_member_up)(node, addr);
            }
            SwimAction::RingRemove { node } => router.remove_ring_node(node),
            SwimAction::Suspect { node } => router.membership().mark_down(node), // 신규 소유권 제외
        }
    }
}

/// SWIM 주기 드라이버 (D45). server가 `tokio::spawn`. 정적 `run_failure_detector`(D23)를 대체.
/// startup에 seed들에게 `SwimJoin` 송신 → 동적 합류. 이후 interval마다 `step` → 액션 실행.
pub async fn run_swim<T: NodeTransport>(
    swim: Arc<Swim>,
    router: Arc<Router<T>>,
    clock: Arc<dyn Clock>,
    on_member_up: MemberUpHook,
    interval_ms: u64,
    seeds: Vec<u64>,
) {
    let mut tick = tokio::time::interval(Duration::from_millis(interval_ms));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        let now = clock.now_ms();
        // H1 robust join: bootstrap 완료 전까지 매 tick seed에 SwimJoin 재전송 — startup 1회 전송이
        // 핸드셰이크 전이라 유실되는 seam을 닫는다. bootstrapped(응답 수신)되면 멈춤.
        if !seeds.is_empty() && !swim.bootstrapped() {
            let join = swim.join_message();
            for s in &seeds {
                router.send_to(*s, join.clone()).await;
            }
        }
        let actions = swim.step(now);
        apply_swim_actions(&router, &on_member_up, now, actions).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> SwimConfig {
        SwimConfig {
            ping_timeout_ms: 100,
            probe_period_ms: 300,
            suspicion_timeout_ms: 500,
            indirect_k: 2,
            gossip_fanout: 2,
            dissemination_count: 4,
            max_piggyback: 8,
            anti_entropy_ticks: 5,
        }
    }

    fn member(node: u64, inc: u64, state: u8) -> SwimMember {
        SwimMember { node_id: node, addr: format!("127.0.0.1:70{node:02}"), incarnation: inc, state }
    }

    /// 모르던 노드의 Alive gossip → 멤버 등록 + RingAdd.
    #[test]
    fn alive_gossip_adds_member_and_ring() {
        let s = Swim::new(1, "127.0.0.1:7001", cfg(), 42);
        let actions = s.handle(2, &NodeMessage::SwimGossip { updates: vec![member(3, 0, 0)] }, 10);
        assert!(actions.iter().any(|a| matches!(a, SwimAction::RingAdd { node: 3, .. })));
        assert_eq!(s.state_of(3), Some(MemberState::Alive));
    }

    /// incarnation 충돌 해소: 낮은 incarnation은 무시, 높은 것만 반영.
    #[test]
    fn incarnation_conflict_resolution() {
        let s = Swim::new(1, "a", cfg(), 1);
        let mut acc = Vec::new();
        {
            let mut g = s.inner.lock().unwrap();
            g.merge_one(&member(5, 3, 0), 0, &mut acc); // Alive inc3
            g.merge_one(&member(5, 2, 1), 0, &mut acc); // Suspect inc2 — 무시(낮음)
            assert_eq!(g.members[&5].state, MemberState::Alive);
            g.merge_one(&member(5, 3, 1), 0, &mut acc); // Suspect inc3 — 같은 inc, Suspect>Alive → 반영
            assert_eq!(g.members[&5].state, MemberState::Suspect);
            g.merge_one(&member(5, 4, 0), 0, &mut acc); // Alive inc4 — 높음 → 반영(부활)
            assert_eq!(g.members[&5].state, MemberState::Alive);
        }
    }

    /// 자기에 대한 Suspect를 보면 incarnation을 올려 반박(refute).
    #[test]
    fn refutes_suspicion_about_self() {
        let s = Swim::new(1, "self", cfg(), 1);
        assert_eq!(s.local_incarnation(), 0);
        s.handle(2, &NodeMessage::SwimGossip { updates: vec![member(1, 0, 1)] }, 5);
        assert_eq!(s.local_incarnation(), 1, "나를 Suspect → incarnation+1 반박");
    }

    /// Suspect → suspicion 타임아웃 → Dead + RingRemove.
    #[test]
    fn suspect_times_out_to_dead() {
        let s = Swim::new(1, "a", cfg(), 1);
        s.handle(2, &NodeMessage::SwimGossip { updates: vec![member(9, 1, 1)] }, 0); // Suspect@0
        assert_eq!(s.state_of(9), Some(MemberState::Suspect));
        let actions = s.step(600); // > suspicion_timeout(500)
        assert!(actions.iter().any(|a| matches!(a, SwimAction::RingRemove { node: 9 })));
        assert_eq!(s.state_of(9), Some(MemberState::Dead));
    }

    /// join 요청을 받은 seed는 합류자를 Alive로 합병 + 전체 스냅샷 회신.
    #[test]
    fn seed_responds_to_join_with_snapshot() {
        let s = Swim::new(1, "127.0.0.1:7001", cfg(), 1);
        s.seed_member(2, "127.0.0.1:7002");
        let actions = s.handle(
            3,
            &NodeMessage::SwimJoin { addr: "127.0.0.1:7003".into(), incarnation: 0 },
            10,
        );
        assert!(actions.iter().any(|a| matches!(a, SwimAction::RingAdd { node: 3, .. })));
        let snap = actions.iter().find_map(|a| match a {
            SwimAction::Send { to: 3, msg: NodeMessage::SwimGossip { updates } } => Some(updates),
            _ => None,
        });
        let snap = snap.expect("seed가 join에 SwimGossip 스냅샷 회신");
        // 스냅샷에 자기(1) + 기존 멤버(2) + 합류자(3) 포함.
        let ids: Vec<u64> = snap.iter().map(|m| m.node_id).collect();
        assert!(ids.contains(&1) && ids.contains(&2) && ids.contains(&3));
    }

    /// 새 탐침은 ping을 보내고, 직접 트래픽 도착 시 성공 처리(다음 step에 같은 노드 재탐침 가능).
    #[test]
    fn probe_pings_and_resolves_on_traffic() {
        let s = Swim::new(1, "a", cfg(), 7);
        s.seed_member(2, "b");
        let actions = s.step(0);
        assert!(actions.iter().any(|a| matches!(
            a,
            SwimAction::Send { to: 2, msg: NodeMessage::SwimPing { .. } }
        )));
        // 노드2가 ack(직접 트래픽) → 탐침 성공.
        s.handle(2, &NodeMessage::SwimAck { seq: 1, updates: vec![] }, 50);
        // 다음 step에 Suspect 전이 없어야 함.
        let a2 = s.step(400);
        assert!(!a2.iter().any(|a| matches!(a, SwimAction::Suspect { .. })));
    }

    /// H1: SwimGossip(join 응답) 수신 전엔 bootstrapped=false, 수신하면 true → join 재전송 멈춤.
    #[test]
    fn bootstrapped_set_on_gossip() {
        let s = Swim::new(3, "127.0.0.1:7003", cfg(), 1);
        assert!(!s.bootstrapped(), "응답 전엔 미합류");
        s.handle(1, &NodeMessage::SwimGossip { updates: vec![member(1, 0, 0)] }, 5);
        assert!(s.bootstrapped(), "seed의 SwimGossip 수신 → 합류 확인");
    }

    /// H2: 라운드로빈 probe — 멤버 수만큼 step하면 모든 멤버가 정확히 한 번씩 ping된다(셔플 1라운드).
    #[test]
    fn round_robin_probes_each_member_once_per_round() {
        let s = Swim::new(1, "a", cfg(), 5);
        for n in [2u64, 3, 4] {
            s.seed_member(n, format!("h{n}"));
        }
        let mut pinged = std::collections::HashSet::new();
        let mut t = 0;
        // 한 라운드(3 멤버) = 3개의 새 probe. 각 probe 사이 ack를 줘서 즉시 다음 대상으로.
        for _ in 0..3 {
            let actions = s.step(t);
            for a in &actions {
                if let SwimAction::Send { to, msg: NodeMessage::SwimPing { .. } } = a {
                    pinged.insert(*to);
                }
            }
            // 직접 ack로 probe 성공 처리 → 다음 step이 새 대상 선정.
            t += 10;
            s.handle(
                actions
                    .iter()
                    .find_map(|a| match a {
                        SwimAction::Send { to, msg: NodeMessage::SwimPing { .. } } => Some(*to),
                        _ => None,
                    })
                    .unwrap(),
                &NodeMessage::SwimAck { seq: 0, updates: vec![] },
                t,
            );
        }
        assert_eq!(pinged, std::collections::HashSet::from([2, 3, 4]), "한 라운드에 전 멤버 1회씩 probe");
    }

    /// H3: anti_entropy_ticks마다 임의 멤버에게 full-snapshot SwimGossip을 push.
    #[test]
    fn anti_entropy_pushes_snapshot_periodically() {
        let s = Swim::new(1, "a", cfg(), 9); // anti_entropy_ticks=5
        s.seed_member(2, "b");
        let mut gossip_steps = 0;
        for i in 1..=5 {
            let actions = s.step(i * 10);
            // 5번째 tick에서 full-snapshot gossip(자기+멤버 포함)이 나와야 함.
            for a in &actions {
                if let SwimAction::Send { msg: NodeMessage::SwimGossip { updates }, .. } = a {
                    if updates.iter().any(|m| m.node_id == 1) {
                        gossip_steps += 1; // self 포함 = full snapshot
                    }
                }
            }
        }
        assert!(gossip_steps >= 1, "anti-entropy 주기에 full-snapshot push 발생");
    }

    /// 직접 ping 무응답 → 간접 ping-req 위임 → 그래도 무응답 → Suspect.
    #[test]
    fn direct_then_indirect_then_suspect() {
        let s = Swim::new(1, "a", cfg(), 3);
        s.seed_member(2, "b");
        s.seed_member(3, "c");
        s.seed_member(4, "d");
        // step@0: 탐침 시작(ping 대상 하나).
        let a0 = s.step(0);
        let target = a0
            .iter()
            .find_map(|a| match a {
                SwimAction::Send { to, msg: NodeMessage::SwimPing { .. } } => Some(*to),
                _ => None,
            })
            .expect("ping 대상");
        // step@150 (>ping_timeout 100): 간접 ping-req 위임.
        let a1 = s.step(150);
        assert!(
            a1.iter().any(|a| matches!(a, SwimAction::Send { msg: NodeMessage::SwimPingReq { .. }, .. })),
            "직접 타임아웃 후 ping-req 위임"
        );
        // step@350 (>probe_period 300): 여전히 무응답 → Suspect.
        let a2 = s.step(350);
        assert!(
            a2.iter().any(|a| matches!(a, SwimAction::Suspect { node } if *node == target)),
            "간접도 실패 → Suspect"
        );
        assert_eq!(s.state_of(target), Some(MemberState::Suspect));
    }
}
