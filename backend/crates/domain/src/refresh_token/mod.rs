//! Refresh 토큰 엔티티 (개념: refresh_token). 순수 데이터 — IO 무의존.
//!
//! D14: refresh 토큰은 **해시만 저장**(원본은 클라에만), 회전(rotation) 시 기존 토큰을
//! `revoked_at` 표시 후 새 토큰을 `rotated_from`으로 연결 → 탈취·재사용 탐지.
//! 만료는 unix seconds로 보관(타임존 라이브러리 비의존, 저장 시 TIMESTAMPTZ로 변환).

use crate::id::{RefreshTokenId, UserId};

/// 저장된 refresh 토큰(활성 여부는 조회 시점에 판정).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefreshToken {
    pub id: RefreshTokenId,
    pub user_id: UserId,
}

/// 신규 refresh 토큰 생성 입력.
#[derive(Clone, Debug)]
pub struct NewRefreshToken {
    pub id: RefreshTokenId,
    pub user_id: UserId,
    /// refresh 토큰의 SHA-256 해시 (원본 비보관).
    pub token_hash: Vec<u8>,
    /// 회전 체인: 이 토큰이 대체한 이전 토큰 (최초 발급 시 None).
    pub rotated_from: Option<RefreshTokenId>,
    /// 만료 시각 (unix seconds).
    pub expires_at_unix: i64,
}
