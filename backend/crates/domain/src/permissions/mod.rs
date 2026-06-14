//! 권한 비트마스크 + 계산 (개념: permissions).
//! 레이아웃/알고리즘: `docs/architecture/permissions.md` (D17).

use bitflags::bitflags;

use crate::id::ChannelId;

bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
    pub struct Permissions: u64 {
        const CREATE_INVITE            = 1 << 0;
        const KICK_MEMBERS             = 1 << 1;
        const BAN_MEMBERS              = 1 << 2;
        const ADMINISTRATOR            = 1 << 3;
        const MANAGE_CHANNELS          = 1 << 4;
        const MANAGE_GUILD             = 1 << 5;
        const ADD_REACTIONS            = 1 << 6;
        const VIEW_AUDIT_LOG           = 1 << 7;
        const VIEW_CHANNEL             = 1 << 10;
        const SEND_MESSAGES            = 1 << 11;
        const SEND_TTS_MESSAGES        = 1 << 12;
        const MANAGE_MESSAGES          = 1 << 13;
        const EMBED_LINKS              = 1 << 14;
        const ATTACH_FILES             = 1 << 15;
        const READ_MESSAGE_HISTORY     = 1 << 16;
        const MENTION_EVERYONE         = 1 << 17;
        const USE_EXTERNAL_EMOJIS      = 1 << 18;
        // 음성: 레이아웃만 정의, 미디어는 범위 밖 (D21)
        const CONNECT                  = 1 << 20;
        const SPEAK                    = 1 << 21;
        const MUTE_MEMBERS             = 1 << 22;
        const DEAFEN_MEMBERS           = 1 << 23;
        const MOVE_MEMBERS             = 1 << 24;
        const CHANGE_NICKNAME          = 1 << 26;
        const MANAGE_NICKNAMES         = 1 << 27;
        const MANAGE_ROLES             = 1 << 28;
        const MANAGE_WEBHOOKS          = 1 << 29;
        const MANAGE_EXPRESSIONS       = 1 << 30;
        const MANAGE_THREADS           = 1 << 34;
        const CREATE_PUBLIC_THREADS    = 1 << 35;
        const SEND_MESSAGES_IN_THREADS = 1 << 38;
    }
}

/// 채널 권한 오버라이드 (allow/deny 쌍).
#[derive(Clone, Copy, Debug, Default)]
pub struct Overwrite {
    pub allow: Permissions,
    pub deny: Permissions,
}

/// 오버라이드 대상 종류 (DB `overwrite_kind` enum 대응).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverwriteKind {
    Role,
    Member,
}

impl OverwriteKind {
    pub fn as_str(self) -> &'static str {
        match self {
            OverwriteKind::Role => "role",
            OverwriteKind::Member => "member",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "role" => Some(OverwriteKind::Role),
            "member" => Some(OverwriteKind::Member),
            _ => None,
        }
    }
}

/// 한 채널의 한 대상(역할/멤버)에 대한 오버라이드 행 (스키마 `channel_overwrites`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelOverwrite {
    pub channel_id: ChannelId,
    /// role_id 또는 user_id (raw). `@everyone`은 realm_id.
    pub target_id: u64,
    pub kind: OverwriteKind,
    pub allow: Permissions,
    pub deny: Permissions,
}

impl Permissions {
    /// 오버라이드 1개 적용: `(self & !deny) | allow`.
    pub fn apply(self, ow: Overwrite) -> Self {
        (self & !ow.deny) | ow.allow
    }

    /// 새 길드 생성 시 `@everyone` 역할의 기본 권한 (일반 멤버가 채팅·참여 가능한 최소 세트).
    pub fn default_everyone() -> Self {
        Permissions::VIEW_CHANNEL
            | Permissions::SEND_MESSAGES
            | Permissions::READ_MESSAGE_HISTORY
            | Permissions::ADD_REACTIONS
            | Permissions::CREATE_INVITE
            | Permissions::CHANGE_NICKNAME
            | Permissions::CONNECT
            | Permissions::SPEAK
    }
}

/// 길드(채널 무관) 유효 권한 (D17). 채널 오버라이드 없이 @everyone + 역할 OR + Admin/owner 단축.
/// 채널 컨텍스트는 [`effective_channel_permissions`]를 쓴다.
pub fn compute_guild_permissions(
    is_owner: bool,
    everyone: Permissions,
    roles: &[Permissions],
) -> Permissions {
    compute_channel_permissions(
        is_owner,
        everyone,
        roles,
        Overwrite::default(),
        &[],
        Overwrite::default(),
    )
}

/// 채널 컨텍스트 유효 권한 (D17) — 오버라이드 목록에서 대상별로 골라 [`compute_channel_permissions`]에 적용.
/// `member_roles` = 멤버의 (비-@everyone) (role_id, perms). `@everyone` 오버라이드는 target_id==realm_id로 매칭.
pub fn effective_channel_permissions(
    is_owner: bool,
    realm_id: u64,
    user_id: u64,
    everyone: Permissions,
    member_roles: &[(u64, Permissions)],
    overwrites: &[ChannelOverwrite],
) -> Permissions {
    let find = |tid: u64| {
        overwrites.iter().find(|o| o.target_id == tid).map(|o| Overwrite { allow: o.allow, deny: o.deny })
    };
    let everyone_ow = find(realm_id).unwrap_or_default();
    let role_perms: Vec<Permissions> = member_roles.iter().map(|(_, p)| *p).collect();
    let role_ows: Vec<Overwrite> = member_roles.iter().filter_map(|(id, _)| find(*id)).collect();
    let member_ow = find(user_id).unwrap_or_default();
    compute_channel_permissions(is_owner, everyone, &role_perms, everyone_ow, &role_ows, member_ow)
}

/// 채널 컨텍스트 유효 권한 계산 (D17).
/// 순서: 소유자/Admin 단축 → @everyone|역할 → @everyone OW → 역할 OW(deny→allow) → 멤버 OW.
pub fn compute_channel_permissions(
    is_owner: bool,
    everyone: Permissions,
    roles: &[Permissions],
    everyone_ow: Overwrite,
    role_ows: &[Overwrite],
    member_ow: Overwrite,
) -> Permissions {
    if is_owner {
        return Permissions::all();
    }
    let mut perms = everyone;
    for r in roles {
        perms |= *r;
    }
    if perms.contains(Permissions::ADMINISTRATOR) {
        return Permissions::all();
    }
    perms = perms.apply(everyone_ow);

    let mut role_deny = Permissions::empty();
    let mut role_allow = Permissions::empty();
    for ow in role_ows {
        role_deny |= ow.deny;
        role_allow |= ow.allow;
    }
    perms = perms.apply(Overwrite { allow: role_allow, deny: role_deny });

    perms.apply(member_ow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_and_admin_bypass_overrides() {
        let all = Permissions::all();
        assert_eq!(
            compute_channel_permissions(
                true,
                Permissions::empty(),
                &[],
                Overwrite::default(),
                &[],
                Overwrite::default()
            ),
            all
        );
        let deny_all = Overwrite { allow: Permissions::empty(), deny: Permissions::all() };
        assert_eq!(
            compute_channel_permissions(
                false,
                Permissions::ADMINISTRATOR,
                &[],
                deny_all,
                &[],
                Overwrite::default()
            ),
            all
        );
    }

    fn cow(target_id: u64, allow: Permissions, deny: Permissions) -> ChannelOverwrite {
        ChannelOverwrite {
            channel_id: crate::id::ChannelId(crate::id::Snowflake::from_raw(1)),
            target_id,
            kind: OverwriteKind::Role,
            allow,
            deny,
        }
    }

    /// @everyone 채널 오버라이드가 SEND_MESSAGES를 deny하면 길드에서 허용돼도 그 채널에선 막힌다.
    /// 멤버 오버라이드 allow가 다시 풀어준다 (최우선).
    #[test]
    fn channel_overwrite_denies_then_member_allows() {
        let realm_id = 100u64;
        let user_id = 7u64;
        let everyone = Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES;

        // @everyone 채널 오버라이드: SEND 차단.
        let deny_send = cow(realm_id, Permissions::empty(), Permissions::SEND_MESSAGES);
        let p = effective_channel_permissions(false, realm_id, user_id, everyone, &[], &[deny_send.clone()]);
        assert!(!p.contains(Permissions::SEND_MESSAGES), "채널 @everyone deny → 막힘");
        assert!(p.contains(Permissions::VIEW_CHANNEL));

        // 멤버 오버라이드 allow → 그 멤버만 다시 가능.
        let allow_send = ChannelOverwrite {
            kind: OverwriteKind::Member,
            target_id: user_id,
            allow: Permissions::SEND_MESSAGES,
            deny: Permissions::empty(),
            ..deny_send.clone()
        };
        let p2 = effective_channel_permissions(false, realm_id, user_id, everyone, &[], &[deny_send, allow_send]);
        assert!(p2.contains(Permissions::SEND_MESSAGES), "멤버 allow가 최우선 → 복구");
    }

    /// 역할 오버라이드: 멤버가 가진 역할에 allow가 붙으면 적용된다.
    #[test]
    fn role_overwrite_applies_to_member_with_role() {
        let realm_id = 100u64;
        let user_id = 7u64;
        let role_id = 55u64;
        let everyone = Permissions::VIEW_CHANNEL;
        let role_allow = cow(role_id, Permissions::SEND_MESSAGES, Permissions::empty());
        // 멤버가 role 55 보유.
        let p = effective_channel_permissions(
            false, realm_id, user_id, everyone, &[(role_id, Permissions::empty())], &[role_allow],
        );
        assert!(p.contains(Permissions::SEND_MESSAGES));
    }

    #[test]
    fn member_overwrite_has_highest_precedence() {
        let everyone = Permissions::SEND_MESSAGES | Permissions::VIEW_CHANNEL;
        let member_ow = Overwrite { allow: Permissions::empty(), deny: Permissions::SEND_MESSAGES };
        let p = compute_channel_permissions(
            false,
            everyone,
            &[],
            Overwrite::default(),
            &[],
            member_ow,
        );
        assert!(p.contains(Permissions::VIEW_CHANNEL));
        assert!(!p.contains(Permissions::SEND_MESSAGES));
    }
}
