-- V5: roles + member_roles (Phase 3, D17). 권한 비트마스크는 BIGINT(raw), 계산은 domain.
-- @everyone 역할 규약: roles.id == realm_id (permissions.md §1).
CREATE TABLE IF NOT EXISTS roles (
    id          BIGINT PRIMARY KEY,
    realm_id    BIGINT NOT NULL REFERENCES realms(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    permissions BIGINT NOT NULL DEFAULT 0,
    position    INT NOT NULL DEFAULT 0,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS ix_roles_realm ON roles (realm_id);

CREATE TABLE IF NOT EXISTS member_roles (
    realm_id BIGINT NOT NULL,
    user_id  BIGINT NOT NULL,
    role_id  BIGINT NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
    PRIMARY KEY (realm_id, user_id, role_id),
    FOREIGN KEY (realm_id, user_id) REFERENCES members (realm_id, user_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS ix_member_roles_member ON member_roles (realm_id, user_id);
