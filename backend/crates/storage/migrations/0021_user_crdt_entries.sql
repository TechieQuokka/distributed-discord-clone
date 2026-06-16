-- V21: CRDT 오프라인 동기화 — 유저 동기화 문서 (D49).
--
-- 키별 LWW-Register(value + (ts_ms, node_id) dot). domain `LwwMap`의 영속형. 여러 기기가
-- 오프라인 편집 후 push해도 LWW 가드 upsert((ts,node) 큰 것 채택)로 충돌 없이 수렴.
-- value NULL = 툼스톤(삭제도 LWW). 병합 권위는 domain(LwwMap), DB는 LWW 보존만.

CREATE TABLE user_crdt_entries (
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    key     TEXT   NOT NULL,
    value   TEXT,                       -- NULL = 툼스톤(삭제)
    ts_ms   BIGINT NOT NULL,            -- LWW dot.0 (편집 시각)
    node_id BIGINT NOT NULL,            -- LWW dot.1 (복제본/기기 id, 타이브레이크)
    PRIMARY KEY (user_id, key)
);
