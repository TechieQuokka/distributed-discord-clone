//! 권한 강제 (개념: perm). DB에서 멤버의 유효 권한을 계산해 검사 (D17, permissions.md §4).
//!
//! 신뢰 경계 = 서버. 각 핸들러가 동작 전 필요한 비트를 `require`로 검사 → 실패 시 403.
//! 계산 자체는 domain(순수); 여기선 DB 데이터를 모아 domain 함수에 먹인다.

use domain::id::{ChannelId, RealmId, UserId};
use domain::permissions::{Permissions, compute_guild_permissions, effective_channel_permissions};
use domain::repo::Store;

use crate::error::ApiError;

/// 길드(채널 무관) 유효 권한. @everyone 없으면(구 길드) 기본값으로 호환.
pub async fn effective<S: Store>(
    store: &S,
    realm: RealmId,
    user: UserId,
) -> Result<Permissions, ApiError> {
    let is_owner = store.get_guild(realm).await?.map(|g| g.owner_id == user).unwrap_or(false);
    let everyone = store
        .everyone_permissions(realm)
        .await?
        .map(Permissions::from_bits_truncate)
        .unwrap_or_else(Permissions::default_everyone);
    let roles: Vec<Permissions> = store
        .member_role_permissions(realm, user)
        .await?
        .into_iter()
        .map(Permissions::from_bits_truncate)
        .collect();
    Ok(compute_guild_permissions(is_owner, everyone, &roles))
}

/// 채널 컨텍스트 유효 권한 (오버라이드 적용, D17). `realm`은 channel의 소유 Realm.
pub async fn effective_in_channel<S: Store>(
    store: &S,
    channel_id: ChannelId,
    realm: RealmId,
    user: UserId,
) -> Result<Permissions, ApiError> {
    let is_owner = store.get_guild(realm).await?.map(|g| g.owner_id == user).unwrap_or(false);
    let everyone = store
        .everyone_permissions(realm)
        .await?
        .map(Permissions::from_bits_truncate)
        .unwrap_or_else(Permissions::default_everyone);
    let member_roles: Vec<(u64, Permissions)> = store
        .member_roles_with_ids(realm, user)
        .await?
        .into_iter()
        .map(|(id, bits)| (id, Permissions::from_bits_truncate(bits)))
        .collect();
    let overwrites = store.list_overwrites(channel_id).await?;
    Ok(effective_channel_permissions(
        is_owner,
        realm.0.raw(),
        user.0.raw(),
        everyone,
        &member_roles,
        &overwrites,
    ))
}

/// 멤버이면서 `needed` 권한을 모두 가질 때만 통과, 아니면 403 (길드 컨텍스트).
pub async fn require<S: Store>(
    store: &S,
    realm: RealmId,
    user: UserId,
    needed: Permissions,
) -> Result<(), ApiError> {
    if !store.is_member(realm, user).await? {
        return Err(ApiError::Forbidden);
    }
    if effective(store, realm, user).await?.contains(needed) {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

/// 채널 컨텍스트(오버라이드 적용)에서 `needed`를 모두 가질 때만 통과, 아니면 403.
pub async fn require_in_channel<S: Store>(
    store: &S,
    channel_id: ChannelId,
    realm: RealmId,
    user: UserId,
    needed: Permissions,
) -> Result<(), ApiError> {
    if !store.is_member(realm, user).await? {
        return Err(ApiError::Forbidden);
    }
    if effective_in_channel(store, channel_id, realm, user).await?.contains(needed) {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}
