//! 세션 레지스트리 (개념: hub). 이 노드의 로컬 세션 추적 + 팬아웃 배달 + RESUME 재생.
//!
//! 세션 소유(D9): 클라가 붙은 노드가 그 세션을 보유. 세션 상태(per-session seq + bounded
//! 재생 버퍼 + live sender)는 **소켓 수명보다 오래** Hub에 산다 — 끊겨도 버퍼를 유지해
//! RESUME 재생(D24)이 가능. 끊김은 `detach`(live만 분리, 버퍼·구독 유지) → grace 후 purge.
//! seq 부여·버퍼 적재는 여기서 단일화(세션 소유 노드의 권위, D24).
//!
//! RESUME 자격증명 = CSPRNG resume_token(추측불가, D20). 세션 id만으로는 재개 불가.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::sync::mpsc;

use crate::protocol::{Outgoing, ServerEvent};

/// 세션당 재생 버퍼 상한 (D27 bounded). 초과분은 오래된 것부터 evict → RESUME 시 gap이면 무효.
pub const DEFAULT_REPLAY_CAP: usize = 256;
/// 끊긴(detached) 세션을 RESUME 위해 살려두는 유예. 이후 purge.
const DETACHED_GRACE: Duration = Duration::from_secs(90);

/// CSPRNG 256-bit hex 토큰 (refresh 토큰과 동일 강도, D20).
fn gen_resume_token() -> String {
    let buf: [u8; 32] = rand::random();
    let mut s = String::with_capacity(64);
    for b in buf {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// 한 WS 세션의 영속 상태. 소켓이 끊겨도(live=None) 버퍼·seq는 유지된다.
struct SessionEntry {
    user_id: u64,
    /// 마지막으로 부여한 dispatch seq (D24). 단조 증가.
    seq: u64,
    cap: usize,
    /// 최근 dispatch 프레임 ring (각 프레임은 자기 `s`를 가짐). RESUME 재생원.
    buffer: VecDeque<Outgoing>,
    /// 현재 붙은 소켓의 송신 채널. 끊기면 None (버퍼는 유지).
    live: Option<mpsc::Sender<Outgoing>>,
    /// RESUME 자격증명 (CSPRNG, D20).
    resume_token: String,
    /// detach 시각 — grace purge 판정용.
    detached_at: Option<Instant>,
}

impl SessionEntry {
    /// dispatch 프레임을 버퍼에 적재 (cap 초과 시 오래된 것부터 evict).
    fn buffer_push(&mut self, frame: Outgoing) {
        if self.buffer.len() == self.cap {
            self.buffer.pop_front();
        }
        self.buffer.push_back(frame);
    }

    /// live 소켓 채널로 push. 채널이 가득(느린 클라)하거나 닫혔으면 **live를 drop**(끊김, D27)
    /// → 세션 채널이 닫혀 pump가 종료·소켓 close. 프레임은 이미 버퍼에 있어 RESUME으로 복구.
    fn push_live(&mut self, frame: Outgoing) {
        if let Some(tx) = &self.live
            && tx.try_send(frame).is_err() {
                self.live = None; // backpressure: 느린 세션 분리(버퍼 유지).
            }
    }
}

#[derive(Default)]
struct Inner {
    /// session_id → 세션 상태.
    sessions: HashMap<u64, SessionEntry>,
    /// user_id → 활성 세션 id 집합 (팬아웃 대상). detach돼도 유지(버퍼 계속 적재).
    by_user: HashMap<u64, HashSet<u64>>,
}

/// 노드 로컬 세션 레지스트리. 복제해도 동일 내부를 공유(Arc).
#[derive(Clone, Default)]
pub struct Hub {
    inner: Arc<Mutex<Inner>>,
}

/// RESUME 시도 결과.
pub enum ResumeOutcome {
    /// 재개 성공 — 새 수신 채널 + 놓친 프레임(재생용).
    Resumed { rx: mpsc::Receiver<Outgoing>, replay: Vec<Outgoing>, last_seq: u64 },
    /// 재개 불가 (미지 세션/토큰 불일치/버퍼 밖 gap) → 호출측이 INVALID_SESSION.
    Invalid,
}

impl Hub {
    pub fn new() -> Self {
        Self::default()
    }

    /// IDENTIFY: 새 세션 등록. 아직 by_user에 넣지 않는다 — READY(seq=1)가 먼저 가도록
    /// `activate`를 별도 호출(그 전엔 팬아웃 비대상). (rx, resume_token) 반환.
    pub fn attach(&self, user_id: u64, session_id: u64, cap: usize) -> (mpsc::Receiver<Outgoing>, String) {
        let (tx, rx) = mpsc::channel(cap);
        let token = gen_resume_token();
        let mut inner = self.inner.lock().unwrap();
        inner.purge_expired();
        inner.sessions.insert(
            session_id,
            SessionEntry {
                user_id,
                seq: 0,
                cap,
                buffer: VecDeque::new(),
                live: Some(tx),
                resume_token: token.clone(),
                detached_at: None,
            },
        );
        (rx, token)
    }

    /// 세션을 팬아웃 대상으로 활성화 (READY 전송 후 호출).
    pub fn activate(&self, user_id: u64, session_id: u64) {
        self.inner.lock().unwrap().by_user.entry(user_id).or_default().insert(session_id);
    }

    /// 세션의 소유 유저 id (RESUME 후 presence 전이용 — RESUME은 session_id만 알고 user는 모름).
    pub fn session_user(&self, session_id: u64) -> Option<u64> {
        self.inner.lock().unwrap().sessions.get(&session_id).map(|e| e.user_id)
    }

    /// 이 유저의 **live(소켓 연결된)** 세션 수 — presence 온/오프라인 전이 판정용(D12).
    /// detach된(버퍼만 남은) 세션은 제외 → 모든 소켓이 끊기면 0 = offline 전이.
    pub fn live_count(&self, user_id: u64) -> usize {
        let inner = self.inner.lock().unwrap();
        inner
            .by_user
            .get(&user_id)
            .map(|set| {
                set.iter()
                    .filter(|sid| inner.sessions.get(sid).map(|e| e.live.is_some()).unwrap_or(false))
                    .count()
            })
            .unwrap_or(0)
    }

    /// 단일 세션에 dispatch (READY 등 세션 전용 이벤트). seq 부여 + 버퍼 적재 + live push.
    pub fn dispatch_one(&self, session_id: u64, t: &str, d: Value) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(e) = inner.sessions.get_mut(&session_id) {
            e.seq += 1;
            let frame = Outgoing::dispatch(e.seq, t, d);
            e.buffer_push(frame.clone());
            e.push_live(frame);
        }
    }

    /// 대상 유저들의 모든 세션에 이벤트 배달. 세션마다 seq 부여 + 버퍼 적재.
    /// 느린 세션(채널 가득)은 **연결을 끊는다**(D27): live sender를 drop → 세션 채널이 닫혀
    /// pump 루프가 종료·소켓 close. 프레임은 버퍼에 남아 클라가 재연결+RESUME으로 복구.
    pub fn deliver(&self, user_ids: &[u64], event: &ServerEvent) {
        let mut inner = self.inner.lock().unwrap();
        // user → session_ids 를 먼저 모아 빌림 충돌 회피.
        let mut session_ids: Vec<u64> = Vec::new();
        for u in user_ids {
            if let Some(set) = inner.by_user.get(u) {
                session_ids.extend(set.iter().copied());
            }
        }
        for sid in session_ids {
            if let Some(e) = inner.sessions.get_mut(&sid) {
                e.seq += 1;
                let frame = Outgoing::dispatch(e.seq, &event.t, event.d.clone());
                e.buffer_push(frame.clone());
                e.push_live(frame);
            }
        }
    }

    /// 끊김: live만 분리(버퍼·구독·seq 유지) → grace 동안 RESUME 가능.
    pub fn detach(&self, session_id: u64) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(e) = inner.sessions.get_mut(&session_id) {
            e.live = None;
            e.detached_at = Some(Instant::now());
        }
    }

    /// 세션 완전 제거 (명시적 종료/무효 — 버퍼·구독 폐기).
    pub fn remove(&self, session_id: u64) {
        let mut inner = self.inner.lock().unwrap();
        inner.drop_session(session_id);
    }

    /// RESUME: 토큰·seq 검증 후 재부착. 성공 시 새 채널 + 누락 프레임.
    /// 자격: 세션 존재 + 토큰 일치 + `last_seq`가 버퍼 범위 내(gap 없음).
    pub fn resume(&self, session_id: u64, token: &str, last_seq: u64, cap: usize) -> ResumeOutcome {
        let mut inner = self.inner.lock().unwrap();
        inner.purge_expired();
        let Some(e) = inner.sessions.get_mut(&session_id) else {
            return ResumeOutcome::Invalid;
        };
        // 토큰 불일치 → 재개 거부 (D20).
        if e.resume_token != token {
            return ResumeOutcome::Invalid;
        }
        // 클라가 우리가 보낸 것보다 앞선 seq 주장 → 불일치.
        if last_seq > e.seq {
            return ResumeOutcome::Invalid;
        }
        // 버퍼에 남은 가장 오래된 seq. 비었으면 다음 발행 예정 seq.
        let earliest = e.seq - e.buffer.len() as u64 + 1;
        // 놓친 첫 이벤트(last_seq+1)가 이미 evict됐으면 gap → 재개 불가.
        // (버퍼가 비어도 last_seq == e.seq면 놓친 것 없음 → OK.)
        if !e.buffer.is_empty() && last_seq + 1 < earliest {
            return ResumeOutcome::Invalid;
        }
        if e.buffer.is_empty() && last_seq != e.seq {
            return ResumeOutcome::Invalid;
        }
        // 재부착: 새 채널.
        let (tx, rx) = mpsc::channel(cap);
        e.live = Some(tx);
        e.detached_at = None;
        let replay: Vec<Outgoing> =
            e.buffer.iter().filter(|f| f.s.map(|s| s > last_seq).unwrap_or(false)).cloned().collect();
        ResumeOutcome::Resumed { rx, replay, last_seq: e.seq }
    }
}

// 유저 단위 이벤트 포트(`UserEmitter`) 구현은 `user_route::UserRouter`(D43)로 이전 —
// Hub는 로컬 세션 배달 프리미티브(`deliver`)만 제공하고, 크로스노드 라우팅은 UserRouter가 조합한다.

impl Inner {
    fn drop_session(&mut self, session_id: u64) {
        if let Some(e) = self.sessions.remove(&session_id)
            && let Some(set) = self.by_user.get_mut(&e.user_id) {
                set.remove(&session_id);
                if set.is_empty() {
                    self.by_user.remove(&e.user_id);
                }
            }
    }

    /// grace를 넘긴 detached 세션 정리 (버퍼 무한 증식 방지).
    fn purge_expired(&mut self) {
        let now = Instant::now();
        let expired: Vec<u64> = self
            .sessions
            .iter()
            .filter(|(_, e)| e.detached_at.map(|t| now.duration_since(t) > DETACHED_GRACE).unwrap_or(false))
            .map(|(&sid, _)| sid)
            .collect();
        for sid in expired {
            self.drop_session(sid);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ev(content: &str) -> ServerEvent {
        ServerEvent { t: "MESSAGE_CREATE".into(), d: json!({ "content": content }) }
    }

    /// 배달이 세션별 단조 seq를 부여하고 버퍼에 적재한다.
    #[tokio::test]
    async fn deliver_assigns_monotonic_seq() {
        let hub = Hub::new();
        let (mut rx, _tok) = hub.attach(1, 100, 16);
        hub.activate(1, 100);

        hub.deliver(&[1], &ev("a"));
        hub.deliver(&[1], &ev("b"));

        let f1 = rx.recv().await.unwrap();
        let f2 = rx.recv().await.unwrap();
        assert_eq!(f1.s, Some(1));
        assert_eq!(f2.s, Some(2));
    }

    /// detach 후에도 배달은 버퍼에 쌓이고, RESUME이 놓친 것만 재생한다.
    #[tokio::test]
    async fn resume_replays_missed_events() {
        let hub = Hub::new();
        let (mut rx, token) = hub.attach(1, 100, 16);
        hub.activate(1, 100);

        hub.deliver(&[1], &ev("a")); // seq 1
        let _ = rx.recv().await.unwrap(); // 클라가 seq1까지 받음
        drop(rx);
        hub.detach(100);

        // 끊긴 동안 도착 (버퍼에 적재).
        hub.deliver(&[1], &ev("b")); // seq 2
        hub.deliver(&[1], &ev("c")); // seq 3

        match hub.resume(100, &token, 1, 16) {
            ResumeOutcome::Resumed { replay, last_seq, .. } => {
                assert_eq!(last_seq, 3);
                let seqs: Vec<u64> = replay.iter().filter_map(|f| f.s).collect();
                assert_eq!(seqs, vec![2, 3]); // 놓친 2,3만 재생
            }
            ResumeOutcome::Invalid => panic!("재개 가능해야 함"),
        }
    }

    /// 토큰 불일치는 재개 거부 (D20).
    #[tokio::test]
    async fn resume_rejects_bad_token() {
        let hub = Hub::new();
        let (_rx, _token) = hub.attach(1, 100, 16);
        hub.activate(1, 100);
        assert!(matches!(hub.resume(100, "deadbeef", 0, 16), ResumeOutcome::Invalid));
    }

    /// 버퍼에서 evict된 이벤트가 필요하면 gap → 재개 불가.
    #[tokio::test]
    async fn resume_detects_gap_after_eviction() {
        let hub = Hub::new();
        let cap = 4;
        let (_rx, token) = hub.attach(1, 100, cap);
        hub.activate(1, 100);
        // cap=4 버퍼에 6개 → seq 1,2는 evict, 남은 건 3..6.
        for i in 0..6 {
            hub.deliver(&[1], &ev(&i.to_string()));
        }
        // 클라가 seq1까지 받았다 주장 → 2가 이미 사라짐 → gap.
        assert!(matches!(hub.resume(100, &token, 1, cap), ResumeOutcome::Invalid));
        // seq3까지 받았다면 4,5,6 재생 가능.
        match hub.resume(100, &token, 3, cap) {
            ResumeOutcome::Resumed { replay, .. } => {
                let seqs: Vec<u64> = replay.iter().filter_map(|f| f.s).collect();
                assert_eq!(seqs, vec![4, 5, 6]);
            }
            ResumeOutcome::Invalid => panic!("seq3 이후는 재개 가능해야 함"),
        }
    }

    /// 느린 클라(채널 가득)는 끊긴다(D27): live sender drop → 채널 닫힘. 버퍼는 남아 RESUME 복구.
    #[tokio::test]
    async fn slow_session_is_disconnected_but_buffer_survives() {
        let hub = Hub::new();
        let cap = 2;
        let (mut rx, token) = hub.attach(1, 100, cap);
        hub.activate(1, 100);

        // rx를 안 읽어 채널을 채운다: a,b는 채널에 적재, c/d는 가득 → live drop(끊김).
        for c in ["a", "b", "c", "d"] {
            hub.deliver(&[1], &ev(c));
        }
        // 채널에 버퍼됐던 a,b는 받지만, 이후엔 닫힘(None) → 끊김 확인.
        assert!(rx.recv().await.is_some());
        assert!(rx.recv().await.is_some());
        assert!(rx.recv().await.is_none(), "느린 세션은 끊겨야 함(live drop)");

        // 재생 버퍼(cap2)는 최신 2개(seq3,4) 보유 → seq2까지 받은 클라는 RESUME으로 복구.
        match hub.resume(100, &token, 2, cap) {
            ResumeOutcome::Resumed { replay, .. } => {
                let seqs: Vec<u64> = replay.iter().filter_map(|f| f.s).collect();
                assert_eq!(seqs, vec![3, 4]);
            }
            ResumeOutcome::Invalid => panic!("끊긴 뒤에도 버퍼 내 RESUME은 가능해야 함"),
        }
    }

    /// 미지 세션 RESUME → 무효.
    #[tokio::test]
    async fn resume_unknown_session_invalid() {
        let hub = Hub::new();
        assert!(matches!(hub.resume(999, "x", 0, 16), ResumeOutcome::Invalid));
    }
}
