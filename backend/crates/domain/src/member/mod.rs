//! 멤버 엔티티 (개념: member). Realm 멤버십 — 순수 데이터, IO 무의존. 스키마 02-schema.md §3.
//!
//! 길드/DM/그룹DM 공용(`members`). `roles`는 `member_roles`에서 모은 (비-@everyone) 역할 id.

use crate::id::{RealmId, RoleId, UserId};

/// 한 Realm의 한 멤버.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub realm_id: RealmId,
    pub user_id: UserId,
    /// 길드 내 별명 (없으면 username 사용은 클라 몫).
    pub nick: Option<String>,
    /// 합류 시각 (unix seconds).
    pub joined_at: i64,
    /// 부여된 (비-@everyone) 역할 id 목록.
    pub roles: Vec<RoleId>,
}
