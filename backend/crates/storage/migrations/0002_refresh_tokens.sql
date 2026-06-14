-- V2: Refresh 토큰 테이블 (D14). 청사진 docs/database/02-schema.md §1과 일치.
-- opaque 토큰의 해시(SHA-256)만 저장(원본 비보관). 회전(rotation) + 재사용 탐지(rotated_from 체인).

CREATE TABLE refresh_tokens (
    id            BIGINT PRIMARY KEY,                              -- Snowflake
    user_id       BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash    BYTEA NOT NULL,                                  -- raw 토큰은 절대 저장 안 함
    rotated_from  BIGINT REFERENCES refresh_tokens(id),           -- 재사용 탐지용 체인
    issued_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at    TIMESTAMPTZ NOT NULL,
    revoked_at    TIMESTAMPTZ,
    user_agent    TEXT,
    ip            INET
);
CREATE UNIQUE INDEX uq_refresh_token_hash ON refresh_tokens (token_hash);
CREATE INDEX ix_refresh_user ON refresh_tokens (user_id) WHERE revoked_at IS NULL;
