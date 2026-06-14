//! 권한 비트마스크 + 계산 (개념: permissions).
//! 레이아웃/알고리즘: `docs/architecture/permissions.md` (D17).

use bitflags::bitflags;

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

impl Permissions {
    /// 오버라이드 1개 적용: `(self & !deny) | allow`.
    pub fn apply(self, ow: Overwrite) -> Self {
        (self & !ow.deny) | ow.allow
    }
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
