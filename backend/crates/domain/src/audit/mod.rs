//! 감사 로그 엔티티 (개념: audit). 순수 데이터 — IO 무의존. 스키마 02-schema.md §8 `audit_log_entries`.
//!
//! 길드 관리 행위(채널/역할/멤버/웹훅 변경)를 시간순으로 기록한다. action_type은 i16 코드,
//! `changes`는 변경 상세를 담은 **불투명 JSON 문자열**(생산 엣지가 직렬화 — domain은 serde 무의존, D39와 동형).

use crate::id::{RealmId, Snowflake, UserId};

/// 감사 행위 코드 (Discord audit_log_events의 부분집합 — 구현된 mutation에 대응).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuditAction {
    ChannelCreate,
    RoleCreate,
    MemberRoleUpdate,
    MemberNickUpdate,
    MemberKick,
    WebhookCreate,
    WebhookDelete,
}

impl AuditAction {
    pub fn code(self) -> i16 {
        match self {
            AuditAction::ChannelCreate => 10,
            AuditAction::RoleCreate => 30,
            AuditAction::MemberRoleUpdate => 25,
            AuditAction::MemberNickUpdate => 24,
            AuditAction::MemberKick => 20,
            AuditAction::WebhookCreate => 50,
            AuditAction::WebhookDelete => 52,
        }
    }

    pub fn from_code(c: i16) -> Option<Self> {
        Some(match c {
            10 => AuditAction::ChannelCreate,
            30 => AuditAction::RoleCreate,
            25 => AuditAction::MemberRoleUpdate,
            24 => AuditAction::MemberNickUpdate,
            20 => AuditAction::MemberKick,
            50 => AuditAction::WebhookCreate,
            52 => AuditAction::WebhookDelete,
            _ => return None,
        })
    }
}

/// 저장된 감사 로그 항목.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditEntry {
    pub id: Snowflake,
    pub realm_id: RealmId,
    pub actor_id: Option<UserId>,
    pub action: AuditAction,
    /// 대상 id(채널/역할/유저/웹훅 등, raw). 없을 수 있음.
    pub target_id: Option<u64>,
    /// 변경 상세 JSON 문자열(불투명). 없으면 None.
    pub changes: Option<String>,
}

/// 신규 감사 로그 입력.
#[derive(Clone, Debug)]
pub struct NewAuditEntry {
    pub id: Snowflake,
    pub realm_id: RealmId,
    pub actor_id: UserId,
    pub action: AuditAction,
    pub target_id: Option<u64>,
    pub changes: Option<String>,
}
