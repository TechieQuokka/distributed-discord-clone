-- V7: reactions (Phase 3, D39) — 메시지 리액션. 청사진 02-schema.md §5.
-- 단순화: 유니코드 emoji 1컬럼으로 PK 구성(원안의 nullable emoji_id/emoji_unicode PK는 무효).
--   커스텀 이모지(emoji_id)는 Phase 4 `emojis`와 함께 도입.
CREATE TABLE IF NOT EXISTS reactions (
    message_id   BIGINT NOT NULL,
    user_id      BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    emoji        TEXT NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (message_id, user_id, emoji)
);
CREATE INDEX IF NOT EXISTS ix_reactions_message ON reactions (message_id);
