//! 초대 엔티티 (개념: invite). 순수 데이터 — IO 무의존. 스키마 02-schema.md `invites`.
//!
//! 초대 = 길드(Realm) 합류 토큰. 짧은 `code`로 식별, `max_uses`/`expires_at`로 제한.
//! redeem(사용)하면 멤버 1행 추가 + uses 증가 (storage 트랜잭션).

use crate::id::{ChannelId, RealmId, UserId};

/// 저장된 초대.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invite {
    pub code: String,
    pub realm_id: RealmId,
    pub channel_id: Option<ChannelId>,
    pub inviter_id: Option<UserId>,
    pub uses: i32,
    /// 0 = 무제한.
    pub max_uses: i32,
    /// unix seconds. None = 무기한.
    pub expires_at: Option<i64>,
}

impl Invite {
    /// `now_unix` 기준 사용 가능한가 (만료·소진 아님).
    pub fn is_valid(&self, now_unix: i64) -> bool {
        let not_expired = self.expires_at.map(|e| now_unix < e).unwrap_or(true);
        let not_exhausted = self.max_uses == 0 || self.uses < self.max_uses;
        not_expired && not_exhausted
    }
}

/// 신규 초대 생성 입력. `code`는 호출자(edge)가 발급, `expires_at`은 max_age로부터 계산.
#[derive(Clone, Debug)]
pub struct NewInvite {
    pub code: String,
    pub realm_id: RealmId,
    pub channel_id: Option<ChannelId>,
    pub inviter_id: Option<UserId>,
    pub max_uses: i32,
    pub expires_at: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::Snowflake;

    fn inv(uses: i32, max_uses: i32, expires_at: Option<i64>) -> Invite {
        Invite {
            code: "abc".into(),
            realm_id: RealmId(Snowflake::from_raw(1)),
            channel_id: None,
            inviter_id: None,
            uses,
            max_uses,
            expires_at,
        }
    }

    #[test]
    fn unlimited_invite_always_valid() {
        assert!(inv(100, 0, None).is_valid(1_000));
    }

    #[test]
    fn exhausted_and_expired_are_invalid() {
        assert!(!inv(5, 5, None).is_valid(1_000)); // 소진
        assert!(!inv(0, 0, Some(500)).is_valid(1_000)); // 만료
        assert!(inv(0, 0, Some(2_000)).is_valid(1_000)); // 아직 유효
    }
}
