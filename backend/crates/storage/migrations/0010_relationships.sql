-- Phase 3: 친구·차단 (02-schema §6). Discord식 방향성 행 — A↔B 친구 = 양쪽 행 2개.
-- relation_kind enum은 0001에서 만들지 않았으므로 여기서 생성.
CREATE TYPE relation_kind AS ENUM ('friend', 'pending_in', 'pending_out', 'blocked');

CREATE TABLE relationships (
    user_id    BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    target_id  BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind       relation_kind NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, target_id),
    CHECK (user_id <> target_id)
);
CREATE INDEX ix_relationships_target ON relationships (target_id);
