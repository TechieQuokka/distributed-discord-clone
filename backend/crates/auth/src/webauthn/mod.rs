//! WebAuthn/Passkeys 서버 ceremony (개념: webauthn). D19. **P6 — 크립토는 검증 크레이트 `webauthn-rs`.**
//!
//! register/auth 시작·완료를 얇게 감싼다. ceremony 중간 상태(`PasskeyRegistration`/`PasskeyAuthentication`)는
//! 호출측(rest-api)이 휘발 보관(DB-D5). 자격증명(`Passkey`)은 직렬화해 저장(opaque, storage가 보관).
//! webauthn-rs 타입을 재노출 → rest-api/cli가 auth를 통해서만 webauthn에 접근(crypto 경계 일원화).

use webauthn_rs::prelude::*;

use crate::error::AuthError;

// rest-api/cli가 ceremony 상태·자격증명을 다루기 위한 타입 재노출.
pub use webauthn_rs::prelude::{
    AuthenticationResult, CreationChallengeResponse, Passkey, PasskeyAuthentication,
    PasskeyRegistration, PublicKeyCredential, RegisterPublicKeyCredential, RequestChallengeResponse,
    Url, Uuid,
};

fn err<E: std::fmt::Display>(e: E) -> AuthError {
    AuthError::WebAuthn(e.to_string())
}

/// 서버측 WebAuthn (Relying Party). rp_id=도메인, rp_origin=클라 origin URL.
pub struct WebauthnService {
    inner: Webauthn,
}

impl WebauthnService {
    pub fn new(rp_id: &str, rp_origin: &str) -> Result<Self, AuthError> {
        let origin = Url::parse(rp_origin).map_err(err)?;
        let inner = WebauthnBuilder::new(rp_id, &origin).map_err(err)?.build().map_err(err)?;
        Ok(Self { inner })
    }

    /// 등록 ceremony 시작. `exclude` = 유저의 기존 자격증명(중복 등록 방지).
    pub fn start_register(
        &self,
        user_id: Uuid,
        name: &str,
        display: &str,
        exclude: &[Passkey],
    ) -> Result<(CreationChallengeResponse, PasskeyRegistration), AuthError> {
        let ids: Vec<CredentialID> = exclude.iter().map(|p| p.cred_id().clone()).collect();
        let excl = if ids.is_empty() { None } else { Some(ids) };
        self.inner.start_passkey_registration(user_id, name, display, excl).map_err(err)
    }

    /// 등록 ceremony 완료 → 저장할 `Passkey`.
    pub fn finish_register(
        &self,
        reg: &RegisterPublicKeyCredential,
        state: &PasskeyRegistration,
    ) -> Result<Passkey, AuthError> {
        self.inner.finish_passkey_registration(reg, state).map_err(err)
    }

    /// 인증 ceremony 시작 — 유저의 등록 자격증명 목록으로 challenge 발급.
    pub fn start_auth(
        &self,
        passkeys: &[Passkey],
    ) -> Result<(RequestChallengeResponse, PasskeyAuthentication), AuthError> {
        self.inner.start_passkey_authentication(passkeys).map_err(err)
    }

    /// 인증 ceremony 완료 → 검증 결과(counter/needs_update 포함). 클론 탐지·서명검증은 라이브러리.
    pub fn finish_auth(
        &self,
        cred: &PublicKeyCredential,
        state: &PasskeyAuthentication,
    ) -> Result<AuthenticationResult, AuthError> {
        self.inner.finish_passkey_authentication(cred, state).map_err(err)
    }
}

/// Snowflake u64 → 결정론 Uuid (webauthn `user_unique_id`).
pub fn user_uuid(snowflake: u64) -> Uuid {
    Uuid::from_u128(snowflake as u128)
}

/// `Passkey`의 자격증명 id 바이트 (저장/조회 키).
pub fn cred_id_bytes(p: &Passkey) -> Vec<u8> {
    p.cred_id().as_ref().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use webauthn_authenticator_rs::WebauthnAuthenticator;
    use webauthn_authenticator_rs::softpasskey::SoftPasskey;

    /// 헤드리스 ceremony 라운드트립 (P6 검증): SoftPasskey로 register→auth 서명검증이 통과.
    #[test]
    fn register_then_authenticate_roundtrip() {
        let svc = WebauthnService::new("localhost", "http://localhost:8080").unwrap();
        let origin = Url::parse("http://localhost:8080").unwrap();
        let mut wa = WebauthnAuthenticator::new(SoftPasskey::new(true));

        // 등록.
        let (ccr, reg_state) = svc.start_register(user_uuid(12345), "alice", "Alice", &[]).unwrap();
        let rpkc = wa.do_registration(origin.clone(), ccr).expect("soft register");
        let passkey = svc.finish_register(&rpkc, &reg_state).unwrap();

        // 인증 — 등록한 자격증명으로 challenge에 서명.
        let (rcr, auth_state) = svc.start_auth(std::slice::from_ref(&passkey)).unwrap();
        let pkc = wa.do_authentication(origin, rcr).expect("soft auth");
        let res = svc.finish_auth(&pkc, &auth_state).unwrap();
        assert_eq!(res.cred_id(), passkey.cred_id(), "인증된 자격증명이 등록한 것과 일치");

        // counter가 등록 후 인증으로 진전 가능(클론 탐지 토대) — needs_update 호출이 동작.
        let _ = res.needs_update();
    }
}
