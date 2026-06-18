//! 이벤트 소싱 — append-only Realm 이벤트 로그 + 순수 프로젝션 (개념: event). D23/D48.
//!
//! **타입화된 도메인 사실(fact)**의 불변 로그. 클라용 JSON(D39 payload)과 **별개** — 이건
//! 상태를 재구성하는 입력(접는 대상)이다. (CQRS: events = write-side fact, projection = read model.)
//!
//! - `RealmEventKind`: 한 Realm에서 일어난 사실(메시지 생성/삭제·멤버 입퇴장 …). 순수 enum.
//! - `RealmProjection`: 이벤트 시퀀스를 **결정론적으로 fold**해 파생 상태를 만든다 — IO 없음(P2),
//!   같은 입력 → 같은 출력(DST 친화, D25). 이벤트 소싱의 핵심(상태 = 이벤트의 함수).
//!
//! 직렬화(코드↔jsonb)는 **storage 어댑터가 소유**(domain은 serde 무의존, P2). domain은
//! 타입과 `code()`만 노출하고, storage가 enum을 (code, payload)로 매핑/역매핑한다.

use std::collections::{BTreeMap, BTreeSet};

use crate::id::{ChannelId, MessageId, RealmId, UserId};

/// Realm에서 일어난 도메인 사실. append-only 로그의 단위 — 불변.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RealmEventKind {
    /// 메시지 생성(D24 persist-then-fanout의 사실화). 프로젝션의 message_count/last_message_id 입력.
    MessageCreated { message_id: MessageId, channel_id: ChannelId, author: UserId },
    /// 메시지 소프트 삭제(D39). message_count 감소.
    MessageDeleted { message_id: MessageId, channel_id: ChannelId },
    /// 멤버 합류(초대 redeem 등). members 집합에 추가.
    MemberJoined { user: UserId },
    /// 멤버 이탈(추방/탈퇴). members 집합에서 제거.
    MemberLeft { user: UserId },
}

impl RealmEventKind {
    /// 저장용 안정 코드 (storage가 smallint로 보관). 변경 금지(append-only 로그 호환).
    pub fn code(&self) -> i16 {
        match self {
            RealmEventKind::MessageCreated { .. } => 1,
            RealmEventKind::MessageDeleted { .. } => 2,
            RealmEventKind::MemberJoined { .. } => 3,
            RealmEventKind::MemberLeft { .. } => 4,
        }
    }
}

/// 로그에서 읽어온 이벤트 1건 (per-realm 단조 seq 포함).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RealmEventRecord {
    pub realm_id: RealmId,
    /// per-realm 단조 시퀀스 (1부터). 순서·재생 커서.
    pub seq: u64,
    pub kind: RealmEventKind,
}

/// 이벤트 시퀀스를 fold한 Realm 파생 상태 (read model). 순수 — 상태 = 이벤트의 함수.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RealmProjection {
    /// 현재 멤버 집합 (Joined - Left).
    pub members: BTreeSet<u64>,
    /// 살아있는 메시지 수 (Created - Deleted, 0 하한).
    pub message_count: u64,
    /// 마지막으로 생성된 메시지 id (D35 캐시 warmup 힌트).
    pub last_message_id: Option<u64>,
    /// **채널별** 마지막 메시지 id (D35 warmup — Realm 액터 콜드 시 채널 last_message_id 복원).
    /// 클라 READY가 "최신으로 점프"에 쓰는 채널별 신호(Discord식). Created 시 채널별 max.
    pub last_message_by_channel: BTreeMap<u64, u64>,
    /// 마지막으로 적용한 seq (재생 커서 — 증분 재생용).
    pub last_seq: u64,
}

impl RealmProjection {
    pub fn new() -> Self {
        Self::default()
    }

    /// 이벤트 1건을 접는다(누적). 같은 이벤트 시퀀스는 항상 같은 상태를 만든다(결정론).
    pub fn apply(&mut self, kind: &RealmEventKind) {
        match kind {
            RealmEventKind::MessageCreated { message_id, channel_id, .. } => {
                self.message_count += 1;
                let raw = message_id.0.raw();
                if self.last_message_id.is_none_or(|cur| raw > cur) {
                    self.last_message_id = Some(raw);
                }
                // 채널별 last id = max (이벤트 순서 무관 결정론). D35 warmup 입력.
                let ch = channel_id.0.raw();
                let slot = self.last_message_by_channel.entry(ch).or_insert(raw);
                *slot = (*slot).max(raw);
            }
            RealmEventKind::MessageDeleted { .. } => {
                self.message_count = self.message_count.saturating_sub(1);
            }
            RealmEventKind::MemberJoined { user } => {
                self.members.insert(user.0.raw());
            }
            RealmEventKind::MemberLeft { user } => {
                self.members.remove(&user.0.raw());
            }
        }
    }

    /// 레코드 1건 적용 + 커서 갱신. (replay에서 사용; seq는 단조 증가 전제.)
    pub fn apply_record(&mut self, rec: &RealmEventRecord) {
        self.apply(&rec.kind);
        self.last_seq = rec.seq;
    }

    /// 이벤트 로그를 처음부터 재생해 상태를 재구성 (rehydrate, D23/D35).
    pub fn replay(events: &[RealmEventRecord]) -> Self {
        let mut p = Self::new();
        for rec in events {
            p.apply_record(rec);
        }
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::Snowflake;

    fn mid(n: u64) -> MessageId {
        MessageId(Snowflake::from_raw(n))
    }
    fn cid(n: u64) -> ChannelId {
        ChannelId(Snowflake::from_raw(n))
    }
    fn uid(n: u64) -> UserId {
        UserId(Snowflake::from_raw(n))
    }
    fn rec(seq: u64, kind: RealmEventKind) -> RealmEventRecord {
        RealmEventRecord { realm_id: RealmId(Snowflake::from_raw(1)), seq, kind }
    }

    #[test]
    fn projection_folds_message_and_member_events() {
        let log = vec![
            rec(1, RealmEventKind::MemberJoined { user: uid(10) }),
            rec(2, RealmEventKind::MemberJoined { user: uid(20) }),
            rec(3, RealmEventKind::MessageCreated { message_id: mid(100), channel_id: cid(5), author: uid(10) }),
            rec(4, RealmEventKind::MessageCreated { message_id: mid(200), channel_id: cid(5), author: uid(20) }),
            rec(5, RealmEventKind::MessageDeleted { message_id: mid(100), channel_id: cid(5) }),
            rec(6, RealmEventKind::MemberLeft { user: uid(10) }),
        ];
        let p = RealmProjection::replay(&log);
        assert_eq!(p.members, BTreeSet::from([20]), "10 이탈 → 20만 남음");
        assert_eq!(p.message_count, 1, "생성 2 - 삭제 1");
        assert_eq!(p.last_message_id, Some(200), "마지막 생성 id");
        assert_eq!(p.last_message_by_channel, BTreeMap::from([(5, 200)]), "채널 5의 last id=200 (D35 warmup)");
        assert_eq!(p.last_seq, 6);
    }

    /// 결정론: 같은 로그를 두 번 재생하면 항상 같은 상태 (이벤트 소싱 불변식, DST 친화).
    #[test]
    fn replay_is_deterministic() {
        let log = vec![
            rec(1, RealmEventKind::MessageCreated { message_id: mid(1), channel_id: cid(1), author: uid(1) }),
            rec(2, RealmEventKind::MemberJoined { user: uid(2) }),
        ];
        assert_eq!(RealmProjection::replay(&log), RealmProjection::replay(&log));
    }

    /// 증분 재생(체크포인트 이후만 적용)이 전체 재생과 같은 상태로 수렴.
    #[test]
    fn incremental_replay_matches_full() {
        let head = vec![rec(1, RealmEventKind::MemberJoined { user: uid(1) })];
        let tail = vec![rec(2, RealmEventKind::MessageCreated { message_id: mid(9), channel_id: cid(1), author: uid(1) })];
        let mut incremental = RealmProjection::replay(&head);
        for r in &tail {
            incremental.apply_record(r);
        }
        let full = RealmProjection::replay(&[head, tail].concat());
        assert_eq!(incremental, full);
    }

    #[test]
    fn delete_never_underflows() {
        let p = RealmProjection::replay(&[rec(1, RealmEventKind::MessageDeleted { message_id: mid(1), channel_id: cid(1) })]);
        assert_eq!(p.message_count, 0, "삭제만 와도 0 하한(언더플로 없음)");
    }
}
