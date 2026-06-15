# 권한 시스템 — 비트마스크 레이아웃 & 계산

> Discord식 권한. 64비트 비트마스크(`BIGINT`). 결정 D17, 스키마 `roles.permissions`/`channel_overwrites`.
> 계산은 `domain` crate(순수 로직), 저장은 raw 비트만 (DB-D4).

---

## 1. 표현

- 권한 = **`u64` 비트마스크** (DB는 `BIGINT`). 63비트 사용 가능 → 현재 범위 충분. 초과 시 `NUMERIC`/`bytea` 승격 (DB 규약).
- `roles.permissions` = 역할의 기본 권한.
- `channel_overwrites.allow` / `.deny` = 채널별 허용/거부 비트.
- **`@everyone` 역할** = `roles.id == realm_id` 규약 (모든 멤버가 암묵 보유).

## 2. 비트 정의 (Discord 정렬)

| 비트 | 값 | 권한 | 분류 |
|---|---|---|---|
| 1<<0 | 0x1 | CREATE_INVITE | 일반 |
| 1<<1 | 0x2 | KICK_MEMBERS | 멤버 |
| 1<<2 | 0x4 | BAN_MEMBERS | 멤버 |
| 1<<3 | 0x8 | **ADMINISTRATOR** | 일반(전권) |
| 1<<4 | 0x10 | MANAGE_CHANNELS | 관리 |
| 1<<5 | 0x20 | MANAGE_GUILD | 관리 |
| 1<<6 | 0x40 | ADD_REACTIONS | 텍스트 |
| 1<<7 | 0x80 | VIEW_AUDIT_LOG | 관리 |
| 1<<10 | 0x400 | VIEW_CHANNEL | 일반 |
| 1<<11 | 0x800 | SEND_MESSAGES | 텍스트 |
| 1<<12 | 0x1000 | SEND_TTS_MESSAGES | 텍스트 |
| 1<<13 | 0x2000 | MANAGE_MESSAGES | 텍스트 |
| 1<<14 | 0x4000 | EMBED_LINKS | 텍스트 |
| 1<<15 | 0x8000 | ATTACH_FILES | 텍스트 |
| 1<<16 | 0x10000 | READ_MESSAGE_HISTORY | 텍스트 |
| 1<<17 | 0x20000 | MENTION_EVERYONE | 텍스트 |
| 1<<18 | 0x40000 | USE_EXTERNAL_EMOJIS | 텍스트 |
| 1<<20 | 0x100000 | CONNECT | 음성※ |
| 1<<21 | 0x200000 | SPEAK | 음성※ |
| 1<<22 | 0x400000 | MUTE_MEMBERS | 음성※ |
| 1<<23 | 0x800000 | DEAFEN_MEMBERS | 음성※ |
| 1<<24 | 0x1000000 | MOVE_MEMBERS | 음성※ |
| 1<<26 | 0x4000000 | CHANGE_NICKNAME | 멤버 |
| 1<<27 | 0x8000000 | MANAGE_NICKNAMES | 멤버 |
| 1<<28 | 0x10000000 | MANAGE_ROLES | 관리 |
| 1<<29 | 0x20000000 | MANAGE_WEBHOOKS | 관리 |
| 1<<30 | 0x40000000 | MANAGE_EXPRESSIONS | 관리 (이모지/스티커) |
| 1<<34 | 0x400000000 | MANAGE_THREADS | 스레드 |
| 1<<35 | 0x800000000 | CREATE_PUBLIC_THREADS | 스레드 |
| 1<<38 | 0x4000000000 | SEND_MESSAGES_IN_THREADS | 스레드 |

> ※ 음성 권한은 **레이아웃만 정의**. 실제 음성 미디어는 범위 밖(D21). 권한 비트는 미래 호환을 위해 예약.
> 비트값은 `domain` crate에 `bitflags`류로 정의 (단일 출처).

## 3. 계산 알고리즘 (D17)

채널 컨텍스트에서 "멤버 M이 권한 P를 갖는가"를 계산하는 순서:

```text
fn compute_permissions(member, guild, channel) -> u64:
    # 1) 소유자 / Administrator 단축
    if member.id == guild.owner_id:
        return ALL

    # 2) 기본 = @everyone 역할
    perms = role(@everyone).permissions

    # 3) 멤버의 역할들을 OR 누적
    for role in member.roles:
        perms |= role.permissions

    # 4) Administrator면 전부 통과 (채널 오버라이드 무시)
    if perms & ADMINISTRATOR:
        return ALL

    # 5) 채널 오버라이드 적용 — 순서 중요
    #    (a) @everyone 오버라이드
    ow = overwrite(channel, @everyone)
    perms = (perms & !ow.deny) | ow.allow

    #    (b) 역할 오버라이드들 — deny 먼저 모두, allow 나중에 모두
    role_deny = 0; role_allow = 0
    for role in member.roles:
        ow = overwrite(channel, role)
        role_deny  |= ow.deny
        role_allow |= ow.allow
    perms = (perms & !role_deny) | role_allow

    #    (c) 멤버별 오버라이드 (최우선)
    ow = overwrite(channel, member)
    perms = (perms & !ow.deny) | ow.allow

    return perms
```

### 불변식 / 주의
- **ADMINISTRATOR는 채널 오버라이드를 건너뛴다** (4단계에서 즉시 ALL).
- **VIEW_CHANNEL 없으면** 그 채널의 다른 권한은 무의미 (사실상 접근 불가).
- 적용 순서는 `@everyone → 역할(deny→allow) → 멤버(deny→allow)` 고정. 멤버 오버라이드가 가장 강함.
- 채널마다 재계산 (캐싱은 후속 최적화 여지, 단 역할/오버라이드 변경 시 무효화 필요).

## 4. 강제(enforcement) 지점
- **REST**: 각 핸들러가 동작 전 필요한 비트 검사 → 실패 시 403 (rest.md 참조).
- **Realm 액터**: REALM_COMMAND 처리 시에도 재검증 (신뢰 경계는 서버; 클라 검사 신뢰 안 함).
- DM/그룹DM Realm: 길드 권한 개념 약함 → 참가자 여부 + 기본 규칙으로 단순화.

## 5. DM/그룹DM에서의 권한
- 1:1 DM: 두 참가자 동등. **차단(relationship=blocked) 시 전송 거부 — 구현됨(D40, Phase 3)**: 어느 한쪽이라도 차단하면 1:1 DM **열기**(rest-api `open_channel`)와 **전송**(gateway `can_send`)에서 `is_blocked_between`으로 거부(403).
- 그룹DM: 소유자(`realms.owner_id`)만 멤버 추가/제거. 그 외 동등. (그룹DM엔 차단 게이팅 미적용 — Discord 동일.)
- DM Realm은 @everyone 역할이 없어 권한 계산이 `default_everyone`으로 폴백 → 멤버면 VIEW/SEND/HISTORY 통과(길드와 동일 경로, D8/P4).
