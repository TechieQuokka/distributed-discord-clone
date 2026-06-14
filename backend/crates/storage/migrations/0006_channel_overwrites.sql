-- V6: channel_overwrites (Phase 3, D17) — 채널별 역할/멤버 권한 오버라이드. 청사진 02-schema.md.
DO $$ BEGIN
    CREATE TYPE overwrite_kind AS ENUM ('role', 'member');
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;

CREATE TABLE IF NOT EXISTS channel_overwrites (
    channel_id   BIGINT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    target_id    BIGINT NOT NULL,                         -- role_id 또는 user_id (@everyone=realm_id)
    target_type  overwrite_kind NOT NULL,
    allow        BIGINT NOT NULL DEFAULT 0,
    deny         BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (channel_id, target_id)
);
