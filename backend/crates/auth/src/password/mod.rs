//! 비밀번호 해싱 (개념: password). Argon2id (D15/P6: 검증된 크레이트).

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};

use crate::error::AuthError;

/// 평문 → Argon2id PHC 문자열.
pub fn hash_password(plain: &str) -> Result<String, AuthError> {
    let salt_bytes: [u8; 16] = rand::random();
    let salt = SaltString::encode_b64(&salt_bytes).map_err(|e| AuthError::Hash(e.to_string()))?;
    let hash = Argon2::default()
        .hash_password(plain.as_bytes(), &salt)
        .map_err(|e| AuthError::Hash(e.to_string()))?;
    Ok(hash.to_string())
}

/// 평문이 PHC 해시와 일치하는지 검증.
pub fn verify_password(plain: &str, phc: &str) -> Result<bool, AuthError> {
    let parsed = PasswordHash::new(phc).map_err(|e| AuthError::Hash(e.to_string()))?;
    Ok(Argon2::default()
        .verify_password(plain.as_bytes(), &parsed)
        .is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_then_verify() {
        let phc = hash_password("hunter2").unwrap();
        assert!(phc.starts_with("$argon2id$"));
        assert!(verify_password("hunter2", &phc).unwrap());
        assert!(!verify_password("wrong", &phc).unwrap());
    }
}
