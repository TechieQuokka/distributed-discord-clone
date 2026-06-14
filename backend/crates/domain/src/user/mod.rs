//! User 엔티티 (개념: user). 순수 데이터 — IO 무의존.

use crate::id::UserId;

/// 저장된 유저.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct User {
    pub id: UserId,
    pub username: String,
    pub global_name: Option<String>,
    pub email: String,
    /// Argon2id PHC 문자열 (D15). 평문 비번은 절대 보관 안 함.
    pub password_hash: String,
    pub is_bot: bool,
}

/// 신규 유저 생성 입력.
#[derive(Clone, Debug)]
pub struct NewUser {
    pub id: UserId,
    pub username: String,
    pub email: String,
    pub password_hash: String,
}
