//! DM/그룹DM 도메인 (개념: dm). 순수 데이터 — IO 무의존. D8/DB-D1/DB-D2, permissions.md §5.
//!
//! Realm 통일 추상(D8/P4): DM·그룹DM도 길드와 같은 `Realm` + 채널 1개다. 따라서 메시징·권한·
//! 분산 팬아웃 경로는 길드와 **동일 코드를 재사용**한다(추가 분기 없음). 여기선 DM 고유의
//! 데이터 모델(중복 방지 페어, 생성 입력, Realm 종류)만 정의한다.

use crate::id::{ChannelId, RealmId, UserId};

/// Realm 종류 (DB `realm_kind` enum 대응).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RealmKind {
    Guild,
    Dm,
    GroupDm,
}

impl RealmKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RealmKind::Guild => "guild",
            RealmKind::Dm => "dm",
            RealmKind::GroupDm => "group_dm",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "guild" => RealmKind::Guild,
            "dm" => RealmKind::Dm,
            "group_dm" => RealmKind::GroupDm,
            _ => return None,
        })
    }
}

/// Realm 메타(요약) — DM/그룹DM 관리·권한 분기용. 길드 전용 속성은 `guild`에 있다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RealmInfo {
    pub id: RealmId,
    pub kind: RealmKind,
    /// 그룹DM 소유자(멤버 추가/제거 권한, permissions.md §5). 길드/1:1 DM은 None.
    pub owner_id: Option<UserId>,
    pub name: Option<String>,
}

/// DM/그룹DM의 채널 핸들(=Realm의 단일 채널). 열기/조회 응답.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DmChannel {
    pub realm_id: RealmId,
    pub channel_id: ChannelId,
    pub kind: RealmKind,
}

/// 1:1 DM 생성 입력. id들은 호출자가 Snowflake로 미리 발급(D11 주입 generator).
#[derive(Clone, Debug)]
pub struct NewDm {
    pub realm_id: RealmId,
    pub channel_id: ChannelId,
    pub user_a: UserId,
    pub user_b: UserId,
}

/// 그룹DM 생성 입력. `members`는 소유자 포함 전체 참가자.
#[derive(Clone, Debug)]
pub struct NewGroupDm {
    pub realm_id: RealmId,
    pub channel_id: ChannelId,
    pub owner: UserId,
    pub name: Option<String>,
    pub members: Vec<UserId>,
}

/// `dm_pairs` 정규화: 항상 `user_lo < user_hi` (DB-D2 CHECK 규약).
/// 같은 두 사람의 DM이 (a,b)/(b,a) 어느 순서로 요청돼도 같은 페어 키로 조회되게 한다.
pub fn order_pair(a: UserId, b: UserId) -> (UserId, UserId) {
    if a.0.raw() <= b.0.raw() { (a, b) } else { (b, a) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::Snowflake;

    fn uid(n: u64) -> UserId {
        UserId(Snowflake::from_raw(n))
    }

    #[test]
    fn order_pair_is_symmetric_and_sorted() {
        let (lo1, hi1) = order_pair(uid(10), uid(3));
        let (lo2, hi2) = order_pair(uid(3), uid(10));
        assert_eq!((lo1, hi1), (uid(3), uid(10)));
        assert_eq!((lo1, hi1), (lo2, hi2), "두 순서 모두 같은 페어 키");
    }

    #[test]
    fn realm_kind_round_trips() {
        for k in [RealmKind::Guild, RealmKind::Dm, RealmKind::GroupDm] {
            assert_eq!(RealmKind::parse(k.as_str()), Some(k));
        }
        assert_eq!(RealmKind::parse("nope"), None);
    }
}
