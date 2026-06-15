-- V12: 메시지 전문검색 (Postgres FTS, Q10 / D28 §5 / 04-partitioning-and-distributed.md §5)
-- tsvector 생성 컬럼(STORED) + GIN 인덱스. config='simple' = 언어 무관(스테밍 없음, 혼합 언어 안전).
-- 파티셔닝(D28) 전환 시 GIN 인덱스는 각 파티션에 상속됨(04 §5).

ALTER TABLE messages
    ADD COLUMN content_tsv tsvector
    GENERATED ALWAYS AS (to_tsvector('simple', content)) STORED;

CREATE INDEX ix_messages_fts ON messages USING GIN (content_tsv);
