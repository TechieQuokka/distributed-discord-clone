//! 가입 봇방지 = Proof-of-Work 챌린지 (개념: pow). D18 (hashcash/mCaptcha 스타일).
//!
//! **stateless 멀티노드 설계**: 챌린지는 서버가 발급한 **PASETO v4.local 토큰**(대칭 인증·암호화 +
//! 만료 내장). 난이도(difficulty)를 토큰 claim(`sub`)에 담아 **위변조 불가**. 어느 노드가 발급하든
//! 같은 키를 공유하면 다른 노드가 검증 가능(D14와 동일 철학 — `POW_SECRET` 공유). 서버는 발급한
//! 챌린지를 저장하지 않는다(DB-D5 휘발) → replay는 만료(PASETO exp)까지 가능하나 비용 게이트는
//! 난이도가 담당(study 범위 허용 seam).
//!
//! **알고리즘**: 클라가 `nonce`를 찾아 `sha256(challenge || ":" || nonce)`의 **선행 0비트 ≥ 난이도**가
//! 되게 한다. crypto 프리미티브는 검증된 크레이트만(P6): 해시=`sha2`, 챌린지 인증=`pasetors`.

use pasetors::Local;
use pasetors::claims::{Claims, ClaimsValidationRules};
use pasetors::keys::{Generate, SymmetricKey};
use pasetors::local;
use pasetors::token::UntrustedToken;
use pasetors::version4::V4;
use sha2::{Digest, Sha256};

use crate::error::AuthError;

/// 기본 난이도(선행 0비트). 2^18 ≈ 26만 해시 ≈ 디버그 빌드 수백 ms. 운영 시 상향 가능.
pub const DEFAULT_DIFFICULTY: u8 = 18;

fn pow_err<E: core::fmt::Display>(e: E) -> AuthError {
    AuthError::Pow(e.to_string())
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(s: &str) -> Result<Vec<u8>, AuthError> {
    if !s.len().is_multiple_of(2) {
        return Err(AuthError::Pow("odd hex length".into()));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(pow_err))
        .collect()
}

/// digest의 선행 0비트 수.
pub fn leading_zero_bits(bytes: &[u8]) -> u32 {
    let mut n = 0;
    for &b in bytes {
        if b == 0 {
            n += 8;
        } else {
            n += b.leading_zeros();
            break;
        }
    }
    n
}

fn pow_digest(challenge: &str, nonce: &str) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(challenge.as_bytes());
    h.update(b":");
    h.update(nonce.as_bytes());
    h.finalize().into()
}

/// 한 (challenge, nonce)가 난이도를 만족하는가. 서버 검증과 클라 솔버가 **공유**하는 단일 판정.
pub fn satisfies(challenge: &str, nonce: &str, difficulty: u8) -> bool {
    leading_zero_bits(&pow_digest(challenge, nonce)) >= difficulty as u32
}

/// 클라/테스트용 솔버: 난이도를 만족하는 nonce를 찾아 반환(십진 문자열). 단일 출처(서버 검증과 동일 해시).
pub fn solve(challenge: &str, difficulty: u8) -> String {
    let mut nonce: u64 = 0;
    loop {
        let s = nonce.to_string();
        if satisfies(challenge, &s, difficulty) {
            return s;
        }
        nonce += 1;
    }
}

/// PoW 챌린지 서명/검증 키 (PASETO v4.local 대칭키). 멀티노드는 공유 필수(`POW_SECRET`).
pub struct PowKeys {
    key: SymmetricKey<V4>,
}

impl PowKeys {
    /// 새 키 생성 (단일노드는 기동마다 새로 — 발급/검증이 같은 프로세스라 무방).
    pub fn generate() -> Result<Self, AuthError> {
        Ok(Self { key: SymmetricKey::<V4>::generate().map_err(pow_err)? })
    }

    /// 키를 hex로 내보냄 (멀티노드 공유용, `server gen-keys`).
    pub fn export_hex(&self) -> String {
        to_hex(self.key.as_bytes())
    }

    /// hex 키에서 복원 (공유 키 로드, `POW_SECRET`).
    pub fn import_hex(secret_hex: &str) -> Result<Self, AuthError> {
        Ok(Self { key: SymmetricKey::<V4>::from(&from_hex(secret_hex)?).map_err(pow_err)? })
    }

    /// 챌린지 발급: 난이도를 인증된 claim(`sub`)에 담은 v4.local 토큰. 만료=PASETO 기본(1h).
    pub fn issue_challenge(&self, difficulty: u8) -> Result<String, AuthError> {
        let mut claims = Claims::new().map_err(pow_err)?;
        claims.subject(&difficulty.to_string()).map_err(pow_err)?;
        local::encrypt(&self.key, &claims, None, None).map_err(pow_err)
    }

    /// 검증: 토큰 진위·만료(PASETO) + `sha256(challenge||":"||nonce)` 선행 0비트 ≥ 토큰에 담긴 난이도.
    /// 난이도는 **토큰에서 디코드한 인증된 값**을 신뢰(클라 제출 난이도 신뢰 안 함).
    pub fn verify(&self, challenge: &str, nonce: &str) -> Result<(), AuthError> {
        let rules = ClaimsValidationRules::new();
        let untrusted =
            UntrustedToken::<Local, V4>::try_from(challenge).map_err(pow_err)?;
        let trusted = local::decrypt(&self.key, &untrusted, &rules, None, None).map_err(pow_err)?;
        let claims = trusted.payload_claims().ok_or_else(|| AuthError::Pow("no claims".into()))?;
        let difficulty: u8 = claims
            .get_claim("sub")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AuthError::Pow("missing difficulty".into()))?
            .parse()
            .map_err(pow_err)?;
        if satisfies(challenge, nonce, difficulty) {
            Ok(())
        } else {
            Err(AuthError::Pow("unsolved challenge".into()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solve_then_verify_roundtrip() {
        let keys = PowKeys::generate().unwrap();
        let challenge = keys.issue_challenge(8).unwrap();
        assert!(challenge.starts_with("v4.local."));
        let nonce = solve(&challenge, 8);
        assert!(keys.verify(&challenge, &nonce).is_ok());
    }

    #[test]
    fn wrong_nonce_rejected() {
        let keys = PowKeys::generate().unwrap();
        let challenge = keys.issue_challenge(12).unwrap();
        // 0은 12비트를 만족할 확률 거의 0 → 미해결로 거부.
        assert!(keys.verify(&challenge, "0").is_err());
    }

    #[test]
    fn tampered_challenge_rejected() {
        let keys = PowKeys::generate().unwrap();
        let mut challenge = keys.issue_challenge(8).unwrap();
        let nonce = solve(&challenge, 8);
        challenge.push('x'); // 토큰 변조 → 복호화 실패.
        assert!(keys.verify(&challenge, &nonce).is_err());
    }

    /// 멀티노드: 공유 키(export→import)로 다른 인스턴스가 같은 챌린지를 검증 (POW_SECRET 공유).
    #[test]
    fn shared_key_verifies_across_instances() {
        let issuer = PowKeys::generate().unwrap();
        let challenge = issuer.issue_challenge(8).unwrap();
        let nonce = solve(&challenge, 8);
        let verifier = PowKeys::import_hex(&issuer.export_hex()).unwrap();
        assert!(verifier.verify(&challenge, &nonce).is_ok());
    }

    #[test]
    fn wrong_key_rejected() {
        let a = PowKeys::generate().unwrap();
        let b = PowKeys::generate().unwrap();
        let challenge = a.issue_challenge(8).unwrap();
        let nonce = solve(&challenge, 8);
        assert!(b.verify(&challenge, &nonce).is_err());
    }

    #[test]
    fn leading_zero_bits_counts() {
        assert_eq!(leading_zero_bits(&[0x00, 0x00, 0xFF]), 16);
        assert_eq!(leading_zero_bits(&[0x0F, 0xFF]), 4);
        assert_eq!(leading_zero_bits(&[0xFF]), 0);
    }
}
