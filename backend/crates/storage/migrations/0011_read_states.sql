-- Phase 3: 읽음 상태 (02-schema §8). 채널별 last_read + 안 읽은 멘션 수.
-- last_read_message_id는 FK 없음(messages는 Phase 4 파티셔닝 대상 — attachments/reactions와 동일 방침).
CREATE TABLE read_states (
    user_id              BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    channel_id           BIGINT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    last_read_message_id BIGINT,
    mention_count        INT NOT NULL DEFAULT 0,
    PRIMARY KEY (user_id, channel_id)
);
