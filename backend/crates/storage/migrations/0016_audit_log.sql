-- V16: 감사 로그 (Phase 4, 02-schema §8). 길드 관리 행위 시간순 기록.
-- id = Snowflake(시간순 정렬 = id DESC). changes = 변경 상세 JSONB(불투명, 생산 엣지 직렬화).

CREATE TABLE audit_log_entries (
    id          BIGINT PRIMARY KEY,
    realm_id    BIGINT NOT NULL REFERENCES realms(id) ON DELETE CASCADE,
    actor_id    BIGINT REFERENCES users(id),
    action_type SMALLINT NOT NULL,
    target_id   BIGINT,
    changes     JSONB,
    reason      TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ix_audit_realm ON audit_log_entries (realm_id, id DESC);
