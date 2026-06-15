# Database — Schema (DDL)

> 도메인별 테이블 정의. 규약·근거는 [01-overview.md](01-overview.md), 분산/파티셔닝은 [04](04-partitioning-and-distributed.md).
> 모든 `id`는 Snowflake `BIGINT`. 이 DDL이 곧 `sqlx` 마이그레이션의 청사진.

---

## 0. ENUM 타입

```sql
CREATE TYPE realm_kind     AS ENUM ('guild', 'dm', 'group_dm');
CREATE TYPE channel_kind   AS ENUM ('text', 'voice', 'category', 'announcement', 'forum', 'thread', 'dm');
CREATE TYPE message_kind   AS ENUM ('default', 'reply', 'system_member_join', 'system_pin', 'system_call', 'thread_starter');
CREATE TYPE relation_kind  AS ENUM ('friend', 'pending_in', 'pending_out', 'blocked');
CREATE TYPE overwrite_kind AS ENUM ('role', 'member');
CREATE TYPE user_status    AS ENUM ('online', 'idle', 'dnd', 'offline');  -- 기본 표시값(휘발 presence와 별개)
```

---

## 1. Identity & Auth

```sql
CREATE TABLE users (
    id              BIGINT PRIMARY KEY,                 -- Snowflake
    username        TEXT NOT NULL,                      -- 로그인/표시용 고유 핸들
    global_name     TEXT,                               -- 표시 이름(닉)
    email           TEXT NOT NULL,
    password_hash   TEXT NOT NULL,                      -- Argon2id (D15)
    avatar          TEXT,                               -- 에셋 경로/해시
    bio             TEXT,
    status          user_status NOT NULL DEFAULT 'offline',
    last_seen_at    TIMESTAMPTZ,                        -- 선택적; presence 원천 아님
    is_bot          BOOLEAN NOT NULL DEFAULT FALSE,
    is_system       BOOLEAN NOT NULL DEFAULT FALSE,
    mfa_totp_secret BYTEA,                              -- 암호화 저장, nullable (D19)
    flags           BIGINT NOT NULL DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at      TIMESTAMPTZ
);
CREATE UNIQUE INDEX uq_users_username ON users (lower(username)) WHERE deleted_at IS NULL;
CREATE UNIQUE INDEX uq_users_email    ON users (lower(email))    WHERE deleted_at IS NULL;

-- Refresh 토큰: opaque, 해시 저장, 회전+재사용 탐지 (D14)
CREATE TABLE refresh_tokens (
    id            BIGINT PRIMARY KEY,
    user_id       BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash    BYTEA NOT NULL,                       -- raw 토큰은 절대 저장 안 함
    rotated_from  BIGINT REFERENCES refresh_tokens(id), -- 재사용 탐지용 체인
    issued_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at    TIMESTAMPTZ NOT NULL,
    revoked_at    TIMESTAMPTZ,
    user_agent    TEXT,
    ip            INET
);
CREATE UNIQUE INDEX uq_refresh_token_hash ON refresh_tokens (token_hash);
CREATE INDEX ix_refresh_user ON refresh_tokens (user_id) WHERE revoked_at IS NULL;

-- MFA 백업 코드 (선택)
CREATE TABLE mfa_backup_codes (
    user_id    BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    code_hash  BYTEA NOT NULL,
    used_at    TIMESTAMPTZ,
    PRIMARY KEY (user_id, code_hash)
);

-- WebAuthn/Passkeys (Phase 5 스트레치)
CREATE TABLE webauthn_credentials (
    id            BIGINT PRIMARY KEY,
    user_id       BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    credential_id BYTEA NOT NULL,
    public_key    BYTEA NOT NULL,
    sign_count    BIGINT NOT NULL DEFAULT 0,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX uq_webauthn_cred ON webauthn_credentials (credential_id);
```

---

## 2. Realms (통일 컨테이너 — DB-D1/D2)

```sql
CREATE TABLE realms (
    id          BIGINT PRIMARY KEY,                     -- Snowflake (1:1 DM도 자기 id)
    kind        realm_kind NOT NULL,
    name        TEXT,                                   -- group_dm 이름 등 (guild는 guilds로)
    owner_id    BIGINT REFERENCES users(id),            -- group_dm 소유자 등
    icon        TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- 길드 전용 메타 (realm_id PK = FK)
CREATE TABLE guilds (
    realm_id              BIGINT PRIMARY KEY REFERENCES realms(id) ON DELETE CASCADE,
    name                  TEXT NOT NULL,
    owner_id              BIGINT NOT NULL REFERENCES users(id),
    icon                  TEXT,
    splash                TEXT,
    description           TEXT,
    verification_level    SMALLINT NOT NULL DEFAULT 0,
    default_notifications SMALLINT NOT NULL DEFAULT 0,
    system_channel_id     BIGINT,                        -- FK는 channels 생성 후 (deferrable)
    afk_channel_id        BIGINT,
    afk_timeout           INT NOT NULL DEFAULT 300,
    vanity_code           TEXT,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX uq_guild_vanity ON guilds (vanity_code) WHERE vanity_code IS NOT NULL;

-- 1:1 DM 중복 방지 조회 테이블 (DB-D2). 항상 user_lo < user_hi
CREATE TABLE dm_pairs (
    user_lo   BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    user_hi   BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    realm_id  BIGINT NOT NULL REFERENCES realms(id) ON DELETE CASCADE,
    PRIMARY KEY (user_lo, user_hi),
    CHECK (user_lo < user_hi)
);
```

---

## 3. Membership & Roles

```sql
-- 모든 Realm의 멤버십 (길드/DM/그룹DM 공용). 길드 전용 필드는 nullable
CREATE TABLE members (
    realm_id    BIGINT NOT NULL REFERENCES realms(id) ON DELETE CASCADE,
    user_id     BIGINT NOT NULL REFERENCES users(id)  ON DELETE CASCADE,
    nick        TEXT,
    joined_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    muted       BOOLEAN NOT NULL DEFAULT FALSE,
    deafened    BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY (realm_id, user_id)
);
CREATE INDEX ix_members_user ON members (user_id);

-- 역할 (길드). @everyone = id == realm_id 규약
CREATE TABLE roles (
    id            BIGINT PRIMARY KEY,
    realm_id      BIGINT NOT NULL REFERENCES realms(id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    color         INT NOT NULL DEFAULT 0,
    position      INT NOT NULL DEFAULT 0,
    permissions   BIGINT NOT NULL DEFAULT 0,             -- 비트마스크 (D17)
    hoist         BOOLEAN NOT NULL DEFAULT FALSE,
    mentionable   BOOLEAN NOT NULL DEFAULT FALSE,
    managed       BOOLEAN NOT NULL DEFAULT FALSE,        -- 봇/통합이 관리
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ix_roles_realm ON roles (realm_id);

-- 멤버 ↔ 역할 (M:N)
CREATE TABLE member_roles (
    realm_id  BIGINT NOT NULL,
    user_id   BIGINT NOT NULL,
    role_id   BIGINT NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
    PRIMARY KEY (realm_id, user_id, role_id),
    FOREIGN KEY (realm_id, user_id) REFERENCES members(realm_id, user_id) ON DELETE CASCADE
);
```

---

## 4. Channels

```sql
CREATE TABLE channels (
    id                  BIGINT PRIMARY KEY,
    realm_id            BIGINT NOT NULL REFERENCES realms(id) ON DELETE CASCADE,
    kind                channel_kind NOT NULL,
    name                TEXT,
    topic               TEXT,
    position            INT NOT NULL DEFAULT 0,
    parent_id           BIGINT REFERENCES channels(id) ON DELETE SET NULL, -- 카테고리/스레드부모
    nsfw                BOOLEAN NOT NULL DEFAULT FALSE,
    rate_limit_per_user INT NOT NULL DEFAULT 0,           -- 슬로우모드(초)
    last_message_id     BIGINT,                           -- 비정규화(최근성)
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at          TIMESTAMPTZ
);
CREATE INDEX ix_channels_realm  ON channels (realm_id) WHERE deleted_at IS NULL;
CREATE INDEX ix_channels_parent ON channels (parent_id);

-- 채널 권한 오버라이드 (DB-D4)
CREATE TABLE channel_overwrites (
    channel_id   BIGINT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    target_id    BIGINT NOT NULL,                         -- role_id 또는 user_id
    target_type  overwrite_kind NOT NULL,
    allow        BIGINT NOT NULL DEFAULT 0,
    deny         BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (channel_id, target_id)
);

-- 스레드 부가정보 (Phase 4) — kind='thread' 채널에 1:1
CREATE TABLE thread_meta (
    channel_id    BIGINT PRIMARY KEY REFERENCES channels(id) ON DELETE CASCADE,
    owner_id      BIGINT REFERENCES users(id),
    archived      BOOLEAN NOT NULL DEFAULT FALSE,
    auto_archive  INT NOT NULL DEFAULT 1440,
    message_count INT NOT NULL DEFAULT 0
);
```

---

## 5. Messages (핵심 — 파티셔닝은 04 문서)

```sql
CREATE TABLE messages (
    id                   BIGINT NOT NULL,                 -- Snowflake (PK는 04에서 파티셔닝 고려)
    channel_id           BIGINT NOT NULL,                 -- FK는 파티셔닝 때 주의(04)
    realm_id             BIGINT NOT NULL,                 -- 비정규화: 파티션/라우팅 키
    author_id            BIGINT NOT NULL REFERENCES users(id),
    kind                 message_kind NOT NULL DEFAULT 'default',
    content              TEXT NOT NULL DEFAULT '',
    embeds               JSONB,                           -- 중첩 임베드
    reference_message_id BIGINT,                          -- 답장 대상
    nonce                TEXT,                            -- 멱등성 (D34/DB-D6)
    pinned               BOOLEAN NOT NULL DEFAULT FALSE,
    tts                  BOOLEAN NOT NULL DEFAULT FALSE,
    flags                BIGINT NOT NULL DEFAULT 0,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    edited_at            TIMESTAMPTZ,
    deleted_at           TIMESTAMPTZ,                      -- 소프트 삭제
    PRIMARY KEY (id)
);
CREATE INDEX ix_messages_channel ON messages (channel_id, id DESC);  -- 히스토리 페이지네이션(D38)
CREATE UNIQUE INDEX uq_messages_nonce ON messages (channel_id, author_id, nonce)
    WHERE nonce IS NOT NULL;

CREATE TABLE attachments (
    id           BIGINT PRIMARY KEY,
    message_id   BIGINT NOT NULL,                         -- FK 주의(파티션, 04)
    filename     TEXT NOT NULL,
    size_bytes   BIGINT NOT NULL,
    content_type TEXT,
    url          TEXT NOT NULL,                           -- 로컬 FS 경로 (D37)
    width        INT,
    height       INT
);
CREATE INDEX ix_attachments_message ON attachments (message_id);

-- 리액션: 유저별 행, 카운트는 집계
-- ※ 구현 단순화 (V7, Phase 3): 유니코드 이모지 1컬럼(`emoji TEXT`)으로 PK 구성.
--   원안의 PK `(message_id, user_id, emoji_id, emoji_unicode)`는 nullable 컬럼을 PK에 넣어
--   Postgres에서 무효(PK 컬럼은 NOT NULL) → 커스텀 이모지(emoji_id)는 Phase 4 `emojis`와 함께 도입.
CREATE TABLE reactions (
    message_id     BIGINT NOT NULL,
    user_id        BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    emoji          TEXT NOT NULL,                         -- 유니코드 이모지 (커스텀 emoji_id는 Phase 4)
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (message_id, user_id, emoji)
);
CREATE INDEX ix_reactions_message ON reactions (message_id);

-- 멘션 인덱스 ("나를 멘션한 메시지" 빠른 조회)
-- ※ 구현 단순화 (V8, Phase 3): 유저 멘션만 `(message_id, user_id)` PK.
--   원안의 `(message_id, user_id, role_id)`는 nullable 컬럼을 PK에 넣어 무효 → 역할 멘션은 Phase 4.
CREATE TABLE message_mentions (
    message_id  BIGINT NOT NULL,
    user_id     BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    PRIMARY KEY (message_id, user_id)
);
CREATE INDEX ix_message_mentions_user ON message_mentions (user_id);
```

---

## 6. Social

```sql
-- 친구/차단 (방향성 행)
CREATE TABLE relationships (
    user_id    BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    target_id  BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind       relation_kind NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, target_id),
    CHECK (user_id <> target_id)
);

CREATE TABLE guild_bans (
    realm_id   BIGINT NOT NULL REFERENCES realms(id) ON DELETE CASCADE,
    user_id    BIGINT NOT NULL REFERENCES users(id)  ON DELETE CASCADE,
    reason     TEXT,
    banned_by  BIGINT REFERENCES users(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (realm_id, user_id)
);

CREATE TABLE invites (
    code       TEXT PRIMARY KEY,                          -- 짧은 코드
    realm_id   BIGINT NOT NULL REFERENCES realms(id) ON DELETE CASCADE,
    channel_id BIGINT REFERENCES channels(id) ON DELETE SET NULL,
    inviter_id BIGINT REFERENCES users(id),
    uses       INT NOT NULL DEFAULT 0,
    max_uses   INT NOT NULL DEFAULT 0,                    -- 0 = 무제한
    max_age    INT NOT NULL DEFAULT 0,                    -- 초, 0 = 무기한
    temporary  BOOLEAN NOT NULL DEFAULT FALSE,
    expires_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ix_invites_realm ON invites (realm_id);
```

---

## 7. Assets

```sql
CREATE TABLE emojis (
    id          BIGINT PRIMARY KEY,
    realm_id    BIGINT NOT NULL REFERENCES realms(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    image       TEXT NOT NULL,
    animated    BOOLEAN NOT NULL DEFAULT FALSE,
    managed     BOOLEAN NOT NULL DEFAULT FALSE,
    created_by  BIGINT REFERENCES users(id),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ix_emojis_realm ON emojis (realm_id);

CREATE TABLE stickers (
    id          BIGINT PRIMARY KEY,
    realm_id    BIGINT REFERENCES realms(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    tags        TEXT,
    asset       TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

---

## 8. Ops (감사·웹훅·읽음상태)

```sql
CREATE TABLE audit_log_entries (
    id          BIGINT PRIMARY KEY,
    realm_id    BIGINT NOT NULL REFERENCES realms(id) ON DELETE CASCADE,
    actor_id    BIGINT REFERENCES users(id),
    action_type SMALLINT NOT NULL,                        -- 행위 코드
    target_id   BIGINT,
    changes     JSONB,
    reason      TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ix_audit_realm ON audit_log_entries (realm_id, id DESC);

CREATE TABLE webhooks (
    id          BIGINT PRIMARY KEY,
    channel_id  BIGINT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    realm_id    BIGINT NOT NULL REFERENCES realms(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    avatar      TEXT,
    token_hash  BYTEA NOT NULL,
    creator_id  BIGINT REFERENCES users(id),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- 읽음 상태 / 미읽음 카운트 (Discord UX 필수)
CREATE TABLE read_states (
    user_id              BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    channel_id           BIGINT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    last_read_message_id BIGINT,
    mention_count        INT NOT NULL DEFAULT 0,
    PRIMARY KEY (user_id, channel_id)
);
```

---

## 9. 보강 (스키마 리뷰 추가)

리뷰에서 발견된 누락 보강.

```sql
-- 봇/애플리케이션 (users.is_bot=true 와 1:1). 봇 토큰은 장수명 opaque, 해시 저장
CREATE TABLE applications (
    id           BIGINT PRIMARY KEY,
    owner_id     BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    bot_user_id  BIGINT REFERENCES users(id) ON DELETE SET NULL,
    name         TEXT NOT NULL,
    description  TEXT,
    bot_token_hash BYTEA,                               -- 봇 토큰(해시), 회전 가능
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ix_applications_owner ON applications (owner_id);

-- 유저 환경설정 (테마/로케일 등)
CREATE TABLE user_settings (
    user_id    BIGINT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    theme      TEXT NOT NULL DEFAULT 'dark',
    locale     TEXT NOT NULL DEFAULT 'ko',
    prefs      JSONB NOT NULL DEFAULT '{}'
);

-- 포럼 채널 태그 (Phase 4)
CREATE TABLE forum_tags (
    id          BIGINT PRIMARY KEY,
    channel_id  BIGINT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    emoji       TEXT,
    moderated   BOOLEAN NOT NULL DEFAULT FALSE
);
CREATE INDEX ix_forum_tags_channel ON forum_tags (channel_id);
```

### 추가 미세 보강
- `roles` 에 `icon TEXT` 컬럼 추가 (역할 아이콘) — Phase 3.
- `messages.pinned` 외에 핀 순서가 필요하면 `channel_pins(channel_id, message_id, pinned_at)` 분리 검토 (현재는 bool 유지).

### 리뷰 결과 — 의도적 보류 (지금 안 만듦)
- **scheduled_events**(예약 이벤트), **polls**(투표), **guild_folders**(클라 UI 정렬) — 핵심 아님, 필요 시 후속.
- **voice_states** — 음성 범위 밖(D21).
- **gateway sessions / presence / rate-limit** — 휘발 상태라 DB 미보관 (DB-D5). 정상.

---

## 부록 — FK 지연(deferred) 주의
`guilds.system_channel_id`, `channels.last_message_id`, `messages.reference_message_id` 등 **순환/시점 의존 FK**는 `DEFERRABLE INITIALLY DEFERRED` 또는 애플리케이션 레벨 무결성으로 처리. 파티셔닝 대상(`messages`)을 참조하는 FK(`attachments`, `reactions`)는 [04 문서](04-partitioning-and-distributed.md)에서 별도 결정.
