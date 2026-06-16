-- V20: 이벤트 소싱 — append-only Realm 이벤트 로그 (D23/D48, 이벤트 소싱).
--
-- 타입화된 도메인 사실(fact)의 불변 로그. messages(엔티티 진실)와 **별개**의 사실 스트림 —
-- CQRS: events = write-side fact, RealmProjection(domain) = read model. per-realm 단조 seq.
-- 단일 직렬 소비자(dispatch 드라이버, D24)가 append → seq 경합 없음(앱레벨, nonce D34와 동형).
--
-- 페이로드는 jsonb 대신 **타입화된 nullable bigint 슬롯**으로 — storage serde 무의존 유지(audit와 정합).
--   MessageCreated: message_id, channel_id, user_id(=author)
--   MessageDeleted: message_id, channel_id
--   MemberJoined/Left: user_id
-- realm_id는 논리 키(append-only 로그라 realms FK 생략 — 로그 자체로 독립).

CREATE TABLE realm_events (
    realm_id   BIGINT   NOT NULL,
    seq        BIGINT   NOT NULL,        -- per-realm 단조(1부터). 순서·재생 커서.
    code       SMALLINT NOT NULL,        -- domain RealmEventKind::code() (안정 코드)
    message_id BIGINT,                   -- 메시지 이벤트
    channel_id BIGINT,                   -- 메시지 이벤트
    user_id    BIGINT,                   -- 멤버 이벤트 / MessageCreated author
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (realm_id, seq)          -- 인덱스가 곧 (realm, seq) 재생 순서
);
