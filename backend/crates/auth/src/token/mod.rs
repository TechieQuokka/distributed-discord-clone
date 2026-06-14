//! Access 토큰 (개념: token). PASETO v4.public (Ed25519, D14).
//! 비대칭 → 노드는 공개키로 검증만 (시크릿 공유 불필요).

use pasetors::Public;
use pasetors::claims::{Claims, ClaimsValidationRules};
use pasetors::keys::{AsymmetricKeyPair, AsymmetricPublicKey, AsymmetricSecretKey, Generate};
use pasetors::public;
use pasetors::token::UntrustedToken;
use pasetors::version4::V4;

use crate::error::AuthError;

fn token_err<E: core::fmt::Display>(e: E) -> AuthError {
    AuthError::Token(e.to_string())
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(s: &str) -> Result<Vec<u8>, AuthError> {
    if !s.len().is_multiple_of(2) {
        return Err(AuthError::Token("odd hex length".into()));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(token_err))
        .collect()
}

/// PASETO v4.public 키쌍 보유자.
pub struct TokenKeys {
    keypair: AsymmetricKeyPair<V4>,
}

impl TokenKeys {
    /// 새 키쌍 생성 (프로세스 시작 시. 실제 운영은 영속 키 로드).
    pub fn generate() -> Result<Self, AuthError> {
        Ok(Self {
            keypair: AsymmetricKeyPair::<V4>::generate().map_err(token_err)?,
        })
    }

    /// 키를 hex로 내보냄 → (secret_hex, public_hex). **멀티노드는 모든 노드가 같은 키를 공유**해야
    /// 한 노드가 발급한 access 토큰을 다른 노드가 검증할 수 있다 (D14, 비대칭).
    pub fn export_hex(&self) -> (String, String) {
        (to_hex(self.keypair.secret.as_bytes()), to_hex(self.keypair.public.as_bytes()))
    }

    /// hex 키쌍에서 복원 (공유 키 로드).
    pub fn import_hex(secret_hex: &str, public_hex: &str) -> Result<Self, AuthError> {
        let secret = AsymmetricSecretKey::<V4>::from(&from_hex(secret_hex)?).map_err(token_err)?;
        let public = AsymmetricPublicKey::<V4>::from(&from_hex(public_hex)?).map_err(token_err)?;
        Ok(Self { keypair: AsymmetricKeyPair { secret, public } })
    }

    /// user_id를 subject로 하는 access 토큰 발급.
    pub fn issue_access(&self, user_id: u64) -> Result<String, AuthError> {
        let mut claims = Claims::new().map_err(token_err)?;
        claims.subject(&user_id.to_string()).map_err(token_err)?;
        public::sign(&self.keypair.secret, &claims, None, None).map_err(token_err)
    }

    /// 토큰 검증 → subject(user_id) 반환.
    pub fn verify_access(&self, token: &str) -> Result<u64, AuthError> {
        let rules = ClaimsValidationRules::new();
        let untrusted = UntrustedToken::<Public, V4>::try_from(token).map_err(token_err)?;
        let trusted =
            public::verify(&self.keypair.public, &untrusted, &rules, None, None).map_err(token_err)?;
        let claims = trusted
            .payload_claims()
            .ok_or_else(|| AuthError::Token("no claims".into()))?;
        let sub = claims
            .get_claim("sub")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AuthError::Token("missing sub".into()))?;
        sub.parse::<u64>().map_err(token_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_then_verify() {
        let keys = TokenKeys::generate().unwrap();
        let token = keys.issue_access(123).unwrap();
        assert!(token.starts_with("v4.public."));
        assert_eq!(keys.verify_access(&token).unwrap(), 123);
    }

    #[test]
    fn tampered_token_fails() {
        let keys = TokenKeys::generate().unwrap();
        let mut token = keys.issue_access(1).unwrap();
        token.push('x');
        assert!(keys.verify_access(&token).is_err());
    }

    /// 멀티노드: export→import한 키로 다른 인스턴스가 같은 토큰을 검증 (D14 공유 키).
    #[test]
    fn exported_keys_verify_across_instances() {
        let issuer = TokenKeys::generate().unwrap();
        let token = issuer.issue_access(42).unwrap();
        let (sk, pk) = issuer.export_hex();

        let verifier = TokenKeys::import_hex(&sk, &pk).unwrap();
        assert_eq!(verifier.verify_access(&token).unwrap(), 42);
        // 복원된 키로 발급한 토큰도 원본이 검증 가능(동일 키쌍).
        let token2 = verifier.issue_access(7).unwrap();
        assert_eq!(issuer.verify_access(&token2).unwrap(), 7);
    }

    #[test]
    fn wrong_key_fails() {
        let a = TokenKeys::generate().unwrap();
        let b = TokenKeys::generate().unwrap();
        let token = a.issue_access(7).unwrap();
        assert!(b.verify_access(&token).is_err());
    }
}
