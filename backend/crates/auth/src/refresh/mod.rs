//! Refresh 토큰 (개념: refresh). opaque 랜덤 + SHA-256 해시 저장 (D14).
//! 원본 토큰은 클라에만, DB엔 해시만 → 유출 시 역산 불가.

use sha2::{Digest, Sha256};

use crate::error::AuthError;

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// 새 refresh 토큰 생성 → (원본 토큰, 저장용 해시).
pub fn generate_refresh() -> Result<(String, Vec<u8>), AuthError> {
    let buf: [u8; 32] = rand::random();
    let token = to_hex(&buf);
    let hash = hash_refresh(&token);
    Ok((token, hash))
}

/// refresh 토큰의 저장/조회용 해시.
pub fn hash_refresh(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_token_hash_matches() {
        let (token, hash) = generate_refresh().unwrap();
        assert_eq!(token.len(), 64); // 32바이트 hex
        assert_eq!(hash_refresh(&token), hash);
    }

    #[test]
    fn tokens_are_unique() {
        let (a, _) = generate_refresh().unwrap();
        let (b, _) = generate_refresh().unwrap();
        assert_ne!(a, b);
    }
}
