//! 역할 엔티티 (개념: role). 순수 데이터 — IO 무의존. 스키마 `roles`/D17.
//!
//! `@everyone` 역할 규약: `role.id == realm_id` (모든 멤버 암묵 보유). permissions.md §1.

use crate::id::{RealmId, RoleId};
use crate::permissions::Permissions;

/// 저장된 역할.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Role {
    pub id: RoleId,
    pub realm_id: RealmId,
    pub name: String,
    pub permissions: Permissions,
    pub position: i32,
}

impl Role {
    /// `@everyone` 역할인가 (id == realm_id 규약).
    pub fn is_everyone(&self) -> bool {
        self.id.0.raw() == self.realm_id.0.raw()
    }
}

/// 신규 역할 생성 입력. id는 호출자가 Snowflake로 발급(@everyone은 realm_id를 그대로).
#[derive(Clone, Debug)]
pub struct NewRole {
    pub id: RoleId,
    pub realm_id: RealmId,
    pub name: String,
    pub permissions: Permissions,
}
