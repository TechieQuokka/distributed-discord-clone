//! WebAuthn 자격증명 (개념: webauthn). D19 Passkeys. 순수 데이터 — **webauthn-rs 무의존(P2)**.
//!
//! 라이브러리 단위(`Passkey`)는 opaque `passkey_json` 문자열로만 안다(공개키/counter는 그 안에 캡슐화).
//! crypto/직렬화는 auth crate(webauthn-rs, P6)가 담당. domain은 저장 포트만 선언.

use crate::id::UserId;

/// 신규 자격증명 저장 입력 (등록 ceremony finish 후).
#[derive(Clone, Debug)]
pub struct NewWebAuthnCredential {
    pub id: u64,                 // Snowflake PK
    pub user_id: UserId,
    pub credential_id: Vec<u8>,  // 자격증명 id (exclude/조회)
    pub passkey_json: String,    // webauthn-rs Passkey 직렬화(불투명)
}

/// 저장된 자격증명 (인증 ceremony 로드 + register exclude용).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebAuthnCredential {
    pub credential_id: Vec<u8>,
    pub passkey_json: String,
}
