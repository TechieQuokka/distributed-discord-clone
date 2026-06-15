-- V13: 스레드 부가정보 (Phase 4, 02-schema §4). kind='thread' 채널에 1:1.
-- 스레드 자체는 `channels`(kind='thread', parent_id=부모 채널) 한 행 — 메시징/팬아웃/권한은
-- 길드 채널과 동일 경로 재사용(P4). thread_meta는 소유자/아카이브/자동아카이브만 보강한다.
-- message_count는 읽기 시 messages에서 집계(쓰기 경로 결합 회피) → 컬럼은 스키마 충실성용(미사용).

CREATE TABLE thread_meta (
    channel_id    BIGINT PRIMARY KEY REFERENCES channels(id) ON DELETE CASCADE,
    owner_id      BIGINT REFERENCES users(id),
    archived      BOOLEAN NOT NULL DEFAULT FALSE,
    auto_archive  INT NOT NULL DEFAULT 1440,
    message_count INT NOT NULL DEFAULT 0
);
