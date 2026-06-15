-- V8: message_mentions (Phase 3, D39) — 유저 멘션 인덱스. 청사진 02-schema.md §5.
-- 단순화: 유저 멘션만 (message_id, user_id) PK. 역할 멘션(role_id)은 Phase 4.
CREATE TABLE IF NOT EXISTS message_mentions (
    message_id   BIGINT NOT NULL,
    user_id      BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    PRIMARY KEY (message_id, user_id)
);
CREATE INDEX IF NOT EXISTS ix_message_mentions_user ON message_mentions (user_id);
