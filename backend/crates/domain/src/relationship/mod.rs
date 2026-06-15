//! 친구·차단 도메인 (개념: relationship). 순수 데이터 — IO 무의존. 스키마 02-schema.md §6.
//!
//! Discord식 **방향성 행**: A↔B 친구는 양쪽 행(A→B, B→A) 2개로 표현한다.
//! - 친구 요청 A→B: A행 `PendingOut`, B행 `PendingIn`.
//! - 수락: 양쪽 `Friend`.
//! - 차단 A→B: A행 `Blocked`, B행은 제거(상대는 관계 없음으로 보임).
//!
//! 상태 전이의 원자성(두 행)은 storage 트랜잭션이 보장한다.

use crate::id::UserId;

/// 관계 종류 (DB `relation_kind` enum 대응). 방향성(내 행 기준).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelationKind {
    Friend,
    /// 상대가 나에게 보낸 요청(내가 받은) — 수락 대기.
    PendingIn,
    /// 내가 상대에게 보낸 요청 — 상대 수락 대기.
    PendingOut,
    Blocked,
}

impl RelationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RelationKind::Friend => "friend",
            RelationKind::PendingIn => "pending_in",
            RelationKind::PendingOut => "pending_out",
            RelationKind::Blocked => "blocked",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "friend" => RelationKind::Friend,
            "pending_in" => RelationKind::PendingIn,
            "pending_out" => RelationKind::PendingOut,
            "blocked" => RelationKind::Blocked,
            _ => return None,
        })
    }

    /// 상대 행의 종류 — 내 행이 이 종류일 때 상대가 보는 관계. (요청은 방향이 뒤집힌다.)
    pub fn mirror(self) -> Self {
        match self {
            RelationKind::Friend => RelationKind::Friend,
            RelationKind::PendingIn => RelationKind::PendingOut,
            RelationKind::PendingOut => RelationKind::PendingIn,
            RelationKind::Blocked => RelationKind::Blocked,
        }
    }
}

/// 한 방향 관계 행 (user_id → target_id).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relationship {
    pub user_id: UserId,
    pub target_id: UserId,
    pub kind: RelationKind,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_round_trips() {
        for k in [
            RelationKind::Friend,
            RelationKind::PendingIn,
            RelationKind::PendingOut,
            RelationKind::Blocked,
        ] {
            assert_eq!(RelationKind::parse(k.as_str()), Some(k));
        }
        assert_eq!(RelationKind::parse("nope"), None);
    }

    #[test]
    fn mirror_flips_pending_only() {
        assert_eq!(RelationKind::PendingOut.mirror(), RelationKind::PendingIn);
        assert_eq!(RelationKind::PendingIn.mirror(), RelationKind::PendingOut);
        assert_eq!(RelationKind::Friend.mirror(), RelationKind::Friend);
        assert_eq!(RelationKind::Blocked.mirror(), RelationKind::Blocked);
    }
}
