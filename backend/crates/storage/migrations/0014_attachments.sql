-- V14: 메시지 첨부 (Phase 4, D37, 02-schema §5). 메타데이터만 — 바이트는 BlobStore(로컬 FS).
-- message FK: Phase 1~3은 정식 FK 유지, 파티셔닝(D28) 전환 시 앱레벨로 완화(04 §2).

CREATE TABLE attachments (
    id           BIGINT PRIMARY KEY,                  -- Snowflake
    message_id   BIGINT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    filename     TEXT NOT NULL,
    size_bytes   BIGINT NOT NULL,
    content_type TEXT,
    url          TEXT NOT NULL,                        -- 다운로드 경로 (/attachments/<id>)
    width        INT,
    height       INT
);
CREATE INDEX ix_attachments_message ON attachments (message_id);
