-- V17: 메시지 시간 RANGE 파티셔닝 (D28, 04 §2). 드롭&재생성 — 로컬 study, 기존 메시지 데이터 폐기(승인).
--
-- ⚠ D34 변경: nonce 멱등성을 DB 부분 유니크 인덱스 → **앱레벨 dedup**으로 이전.
--   Postgres는 파티션 테이블의 유니크 인덱스가 파티션 키(id)를 포함하도록 강제 →
--   uq(channel_id,author_id,nonce)는 만들 수 없음. dispatch 드라이버(단일 직렬 소비자)가
--   persist 전 (channel,author,nonce) 존재 검사로 dedup (레이스 없음). decisions D34/D28 갱신.
-- 왜 RANGE(id): id 상위 비트가 시간 → "최근=핫" 지역성. realm 해시는 런타임 라우팅이 이미 처리(04 §2).

-- 1) 의존 FK 제거 + 첨부 비우기(메시지 폐기에 동반).
ALTER TABLE attachments DROP CONSTRAINT attachments_message_id_fkey;
TRUNCATE attachments;

-- 2) 기존 messages 폐기 (데이터 폐기).
DROP TABLE messages;

-- 3) RANGE(id) 파티션 부모 재생성 (V1 컬럼 + V12 content_tsv FTS, nonce 유니크 없음).
CREATE TABLE messages (
    id                   BIGINT NOT NULL,
    channel_id           BIGINT NOT NULL,
    realm_id             BIGINT NOT NULL,
    author_id            BIGINT NOT NULL REFERENCES users(id),
    kind                 message_kind NOT NULL DEFAULT 'default',
    content              TEXT NOT NULL DEFAULT '',
    embeds               JSONB,
    reference_message_id BIGINT,
    nonce                TEXT,
    pinned               BOOLEAN NOT NULL DEFAULT FALSE,
    tts                  BOOLEAN NOT NULL DEFAULT FALSE,
    flags                BIGINT NOT NULL DEFAULT 0,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    edited_at            TIMESTAMPTZ,
    deleted_at           TIMESTAMPTZ,
    content_tsv          tsvector GENERATED ALWAYS AS (to_tsvector('simple', content)) STORED,
    PRIMARY KEY (id)
) PARTITION BY RANGE (id);

-- 인덱스는 부모에 만들면 각 파티션에 상속됨(04 §5).
CREATE INDEX ix_messages_channel ON messages (channel_id, id DESC);  -- 히스토리(D38)
CREATE INDEX ix_messages_fts ON messages USING GIN (content_tsv);    -- FTS(Q10)

-- 4) 월별 파티션. id 경계 = (month_start_ms - EPOCH_MS(1.7e12)) << 22. DEFAULT가 그 밖을 흡수.
--    (작은 id의 과거/테스트 메시지는 DEFAULT로 라우팅 → 신규 월 사전 생성이 운영 작업, 04 §6.)
CREATE TABLE messages_2026_06 PARTITION OF messages FOR VALUES FROM (336685170688000000) TO (347556806656000000);
CREATE TABLE messages_2026_07 PARTITION OF messages FOR VALUES FROM (347556806656000000) TO (358790830489600000);
CREATE TABLE messages_default PARTITION OF messages DEFAULT;

-- 5) 첨부 FK 복원. PG 12+는 파티션 부모 참조 FK + ON DELETE CASCADE 지원 → 04 §2의 (a) 앱레벨 완화 불필요.
ALTER TABLE attachments ADD CONSTRAINT attachments_message_id_fkey
    FOREIGN KEY (message_id) REFERENCES messages(id) ON DELETE CASCADE;
