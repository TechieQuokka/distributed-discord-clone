-- V4: invites (Phase 3) — 길드 합류 토큰. 청사진 docs/database/02-schema.md `invites`.
CREATE TABLE IF NOT EXISTS invites (
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
CREATE INDEX IF NOT EXISTS ix_invites_realm ON invites (realm_id);
