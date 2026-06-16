-- V18: WebAuthn/Passkeys (Phase 5, D19, 02-schema §1). FIDO2 공개키 자격증명.
-- passkey = webauthn-rs Passkey 직렬화(공개키+counter 캡슐화, 불투명 JSONB). credential_id = exclude/조회.

CREATE TABLE webauthn_credentials (
    id            BIGINT PRIMARY KEY,
    user_id       BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    credential_id BYTEA NOT NULL,
    passkey       JSONB NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX uq_webauthn_cred ON webauthn_credentials (credential_id);
CREATE INDEX ix_webauthn_user ON webauthn_credentials (user_id);
