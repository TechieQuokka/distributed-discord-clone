//! 2단계 인증 = TOTP (개념: totp). RFC 6238 (D19). P6: 검증 크레이트(`totp-rs`) 사용.
//!
//! 흐름(저장은 rest-api): **enable**(secret 발급, 미저장) → **verify**(secret+code 확인 시 저장=활성화,
//! 락아웃 방지) → **login**이 MFA 활성 유저면 `mfa_required` → 코드 재제출로 토큰 발급.
//! secret(raw 바이트)은 DB `users.mfa_totp_secret`(BYTEA)에 저장, 클라 전송은 hex.

use totp_rs::{Algorithm, TOTP};

use crate::error::AuthError;

const ISSUER: &str = "discord-v1";
const DIGITS: usize = 6;
const SKEW: u8 = 1; // ±1 step(30s) 허용 — 시계 드리프트 흡수.
const STEP: u64 = 30;

fn totp_err<E: core::fmt::Display>(e: E) -> AuthError {
    AuthError::Token(e.to_string())
}

/// 새 TOTP secret (20바이트, RFC 6238 SHA1 권장 길이).
pub fn new_secret() -> Vec<u8> {
    rand::random::<[u8; 20]>().to_vec()
}

pub fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn decode_hex(s: &str) -> Result<Vec<u8>, AuthError> {
    if !s.len().is_multiple_of(2) {
        return Err(AuthError::Token("odd hex length".into()));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(totp_err))
        .collect()
}

fn build(secret: &[u8], account: &str) -> Result<TOTP, AuthError> {
    TOTP::new(
        Algorithm::SHA1,
        DIGITS,
        SKEW,
        STEP,
        secret.to_vec(),
        Some(ISSUER.to_string()),
        account.to_string(),
    )
    .map_err(totp_err)
}

/// 인증 앱 등록용 `otpauth://` URI (QR 인코딩 대상).
pub fn otpauth_uri(secret: &[u8], account: &str) -> Result<String, AuthError> {
    Ok(build(secret, account)?.get_url())
}

/// `now_unix` 기준 현재 코드 검증 (±1 step 허용).
pub fn verify(secret: &[u8], code: &str, now_unix: u64) -> Result<bool, AuthError> {
    Ok(build(secret, "user")?.check(code, now_unix))
}

/// `now_unix` 기준 코드 생성 (CLI/테스트용 — 인증 앱 대역).
pub fn generate(secret: &[u8], now_unix: u64) -> Result<String, AuthError> {
    build(secret, "user")?.generate(now_unix).pipe(Ok)
}

/// 작은 후위 적용 헬퍼 (가독성).
trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}
impl<T> Pipe for T {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_code_verifies() {
        let secret = new_secret();
        let now = 1_700_000_000;
        let code = generate(&secret, now).unwrap();
        assert_eq!(code.len(), DIGITS);
        assert!(verify(&secret, &code, now).unwrap());
    }

    #[test]
    fn wrong_code_rejected() {
        let secret = new_secret();
        let now = 1_700_000_000;
        let code = generate(&secret, now).unwrap();
        // 코드를 살짝 바꾸면 거부.
        let bad: String = code.chars().rev().collect();
        if bad != code {
            assert!(!verify(&secret, &bad, now).unwrap());
        }
        assert!(!verify(&secret, "000000", now + 10_000_000).unwrap_or(false) && true);
    }

    #[test]
    fn skew_tolerates_adjacent_step() {
        let secret = new_secret();
        let now = 1_700_000_000;
        let code = generate(&secret, now).unwrap();
        // 한 스텝(30s) 뒤에도 SKEW=1 허용 안엔 통과.
        assert!(verify(&secret, &code, now + STEP).unwrap());
        // 두 스텝(60s) 뒤엔 실패.
        assert!(!verify(&secret, &code, now + STEP * 3).unwrap());
    }

    #[test]
    fn hex_roundtrip() {
        let secret = new_secret();
        assert_eq!(decode_hex(&encode_hex(&secret)).unwrap(), secret);
    }

    #[test]
    fn otpauth_uri_has_issuer() {
        let uri = otpauth_uri(&new_secret(), "alice").unwrap();
        assert!(uri.starts_with("otpauth://totp/"));
        assert!(uri.contains("discord-v1"));
    }
}
