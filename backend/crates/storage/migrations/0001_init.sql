-- Phase 0 코어 스키마. 전체 스키마: docs/database/02-schema.md
-- 후속 Phase에서 roles/members/relationships/reactions 등 마이그레이션 추가.

CREATE TYPE realm_kind   AS ENUM ('guild', 'dm', 'group_dm');
CREATE TYPE channel_kind AS ENUM ('text', 'voice', 'category', 'announcement', 'forum', 'thread', 'dm');
CREATE TYPE message_kind AS ENUM ('default', 'reply', 'system_member_join', 'system_pin', 'system_call', 'thread_starter');
CREATE TYPE user_status  AS ENUM ('online', 'idle', 'dnd', 'offline');

-- Identity
CREATE TABLE users (
    id              BIGINT PRIMARY KEY,
    username        TEXT NOT NULL,
    global_name     TEXT,
    email           TEXT NOT NULL,
    password_hash   TEXT NOT NULL,
    avatar          TEXT,
    bio             TEXT,
    status          user_status NOT NULL DEFAULT 'offline',
    last_seen_at    TIMESTAMPTZ,
    is_bot          BOOLEAN NOT NULL DEFAULT FALSE,
    is_system       BOOLEAN NOT NULL DEFAULT FALSE,
    mfa_totp_secret BYTEA,
    flags           BIGINT NOT NULL DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at      TIMESTAMPTZ
);
CREATE UNIQUE INDEX uq_users_username ON users (lower(username)) WHERE deleted_at IS NULL;
CREATE UNIQUE INDEX uq_users_email    ON users (lower(email))    WHERE deleted_at IS NULL;

-- Realms (통일 컨테이너)
CREATE TABLE realms (
    id          BIGINT PRIMARY KEY,
    kind        realm_kind NOT NULL,
    name        TEXT,
    owner_id    BIGINT REFERENCES users(id),
    icon        TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE guilds (
    realm_id              BIGINT PRIMARY KEY REFERENCES realms(id) ON DELETE CASCADE,
    name                  TEXT NOT NULL,
    owner_id              BIGINT NOT NULL REFERENCES users(id),
    icon                  TEXT,
    description           TEXT,
    verification_level    SMALLINT NOT NULL DEFAULT 0,
    default_notifications SMALLINT NOT NULL DEFAULT 0,
    system_channel_id     BIGINT,
    afk_channel_id        BIGINT,
    afk_timeout           INT NOT NULL DEFAULT 300,
    vanity_code           TEXT,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Channels
CREATE TABLE channels (
    id                  BIGINT PRIMARY KEY,
    realm_id            BIGINT NOT NULL REFERENCES realms(id) ON DELETE CASCADE,
    kind                channel_kind NOT NULL,
    name                TEXT,
    topic               TEXT,
    position            INT NOT NULL DEFAULT 0,
    parent_id           BIGINT REFERENCES channels(id) ON DELETE SET NULL,
    nsfw                BOOLEAN NOT NULL DEFAULT FALSE,
    rate_limit_per_user INT NOT NULL DEFAULT 0,
    last_message_id     BIGINT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at          TIMESTAMPTZ
);
CREATE INDEX ix_channels_realm ON channels (realm_id) WHERE deleted_at IS NULL;

-- Messages (파티셔닝은 Phase 4, D28)
CREATE TABLE messages (
    id                   BIGINT PRIMARY KEY,
    channel_id           BIGINT NOT NULL,
    realm_id             BIGINT NOT NULL,
    author_id            BIGINT NOT NULL REFERENCES users(id),
    kind                 message_kind NOT NULL DEFAULT 'default',
    content              TEXT NOT NULL DEFAULT '',
    embeds               JSONB,
    reference_message_id BIGINT,
    nonce                TEXT,
    pinned               BOOLEAN NOT NULL DEFAULT FALSE,
    tts                  BOOLEAN NOT NULL DEFAULT FALSE,
    flags                BIGINT NOT NULL DEFAULT 0,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    edited_at            TIMESTAMPTZ,
    deleted_at           TIMESTAMPTZ
);
CREATE INDEX ix_messages_channel ON messages (channel_id, id DESC);
CREATE UNIQUE INDEX uq_messages_nonce ON messages (channel_id, author_id, nonce) WHERE nonce IS NOT NULL;
