-- Phase 3: 1:1 DM 중복 방지 조회 테이블 (DB-D2). DM/그룹DM 자체는 기존 realms/channels/members 재사용.
-- 1:1 DM도 자기 Snowflake realm_id를 받고(라우팅 해시 통일), "이 두 사람의 DM이 이미 있나"는
-- 여기서 조회. 항상 user_lo < user_hi 로 정규화(양방향 요청이 같은 페어 키로 매핑).
CREATE TABLE dm_pairs (
    user_lo   BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    user_hi   BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    realm_id  BIGINT NOT NULL REFERENCES realms(id) ON DELETE CASCADE,
    PRIMARY KEY (user_lo, user_hi),
    CHECK (user_lo < user_hi)
);
