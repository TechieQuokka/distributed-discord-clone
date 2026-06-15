-- V15: 웹훅 (Phase 4, 02-schema §8). 채널에 토큰으로 메시지 게시.
-- token_hash = SHA-256(opaque 랜덤 토큰) — 원본은 생성 시 1회 반환, DB엔 해시만(D14와 동일 철학).

CREATE TABLE webhooks (
    id          BIGINT PRIMARY KEY,                  -- Snowflake
    channel_id  BIGINT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    realm_id    BIGINT NOT NULL REFERENCES realms(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    avatar      TEXT,
    token_hash  BYTEA NOT NULL,
    creator_id  BIGINT REFERENCES users(id),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ix_webhooks_channel ON webhooks (channel_id);
