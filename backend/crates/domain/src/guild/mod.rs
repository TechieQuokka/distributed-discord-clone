//! 길드 엔티티 (개념: guild). 순수 데이터 — IO 무의존. 스키마 02-schema.md §2.
//!
//! 길드 = `realms`(kind='guild') + `guilds` 메타 + 소유자 `members` 1행. 생성은 한 트랜잭션 (storage).

use crate::id::{RealmId, UserId};

/// 저장된 길드(요약).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Guild {
    pub realm_id: RealmId,
    pub name: String,
    pub owner_id: UserId,
}

/// 신규 길드 생성 입력. realm_id는 호출자가 Snowflake로 미리 발급.
#[derive(Clone, Debug)]
pub struct NewGuild {
    pub realm_id: RealmId,
    pub name: String,
    pub owner_id: UserId,
}
