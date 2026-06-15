//! 스레드 엔티티 (개념: thread). 순수 데이터 — IO 무의존. 스키마 02-schema.md §4 `thread_meta`.
//!
//! 스레드 = `channels`(kind='thread', parent_id=부모 채널) 한 행 + `thread_meta` 보강(P4).
//! 메시징·팬아웃·권한은 길드 채널과 동일 경로를 재사용한다 — 스레드 특수 코드 없음.

use crate::id::{ChannelId, RealmId, UserId};

/// 저장된 스레드 (channels + thread_meta 조인 뷰).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Thread {
    pub id: ChannelId,
    pub realm_id: RealmId,
    pub parent_id: ChannelId,
    pub name: Option<String>,
    pub owner_id: Option<UserId>,
    pub archived: bool,
    pub auto_archive: i32,
    /// 살아있는 메시지 수 (읽기 시 messages에서 집계 — 쓰기 경로 비결합).
    pub message_count: i64,
}

/// 신규 스레드 생성 입력. 부모 채널과 같은 Realm에 kind='thread' 채널 + thread_meta 한 트랜잭션.
#[derive(Clone, Debug)]
pub struct NewThread {
    pub id: ChannelId,
    pub realm_id: RealmId,
    pub parent_id: ChannelId,
    pub name: String,
    pub owner: UserId,
    /// 자동 아카이브(분). 기본 1440(24h).
    pub auto_archive: i32,
}
