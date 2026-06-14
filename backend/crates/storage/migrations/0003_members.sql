-- V3: 멤버십 테이블 (DB-D1). 청사진 docs/database/02-schema.md §3와 일치.
-- 모든 Realm(길드/DM/그룹DM) 공용 멤버십. 자동 구독(D13)·권한 검사의 기준.

CREATE TABLE members (
    realm_id    BIGINT NOT NULL REFERENCES realms(id) ON DELETE CASCADE,
    user_id     BIGINT NOT NULL REFERENCES users(id)  ON DELETE CASCADE,
    nick        TEXT,
    joined_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    muted       BOOLEAN NOT NULL DEFAULT FALSE,
    deafened    BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY (realm_id, user_id)
);
CREATE INDEX ix_members_user ON members (user_id);
