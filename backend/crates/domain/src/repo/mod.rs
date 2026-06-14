//! 리포지토리 port (개념: repo). domain이 trait로 선언, storage가 구현(adapter) (D22).

use core::future::Future;

use crate::channel::{Channel, NewChannel};
use crate::guild::NewGuild;
use crate::id::{ChannelId, MessageId, RealmId, RefreshTokenId, UserId};
use crate::message::{Message, NewMessage};
use crate::refresh_token::{NewRefreshToken, RefreshToken};
use crate::user::{NewUser, User};

#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    #[error("conflict (unique violation)")]
    Conflict,
    #[error("backend error: {0}")]
    Backend(String),
}

/// 유저 저장소 port.
pub trait UserRepository: Send + Sync {
    fn create_user(&self, user: &NewUser) -> impl Future<Output = Result<(), RepoError>> + Send;
    fn find_by_username(
        &self,
        username: &str,
    ) -> impl Future<Output = Result<Option<User>, RepoError>> + Send;
    fn find_by_id(
        &self,
        id: UserId,
    ) -> impl Future<Output = Result<Option<User>, RepoError>> + Send;
}

/// Refresh 토큰 저장소 port (D14). 회전 + 재사용 탐지.
pub trait RefreshTokenRepository: Send + Sync {
    fn create_refresh_token(
        &self,
        token: &NewRefreshToken,
    ) -> impl Future<Output = Result<(), RepoError>> + Send;

    /// 폐기·만료되지 않은(활성) 토큰을 해시로 조회. `now_unix`로 만료 판정.
    fn find_active(
        &self,
        token_hash: &[u8],
        now_unix: i64,
    ) -> impl Future<Output = Result<Option<RefreshToken>, RepoError>> + Send;

    /// 상태 무관하게 해시로 조회 (재사용 탐지용 — 이미 폐기된 토큰 제시 감지).
    fn find_by_hash(
        &self,
        token_hash: &[u8],
    ) -> impl Future<Output = Result<Option<RefreshToken>, RepoError>> + Send;

    /// 토큰 1개 폐기(revoked_at 표시). 멱등.
    fn revoke(
        &self,
        id: RefreshTokenId,
        now_unix: i64,
    ) -> impl Future<Output = Result<(), RepoError>> + Send;

    /// 유저의 모든 활성 토큰 폐기 (재사용 탐지 시 체인 전체 무효화, D14).
    fn revoke_all_for_user(
        &self,
        user_id: UserId,
        now_unix: i64,
    ) -> impl Future<Output = Result<(), RepoError>> + Send;
}

/// 길드/멤버십 저장소 port (DB-D1). 길드 = realm+guild+member 한 트랜잭션.
pub trait GuildRepository: Send + Sync {
    fn create_guild(&self, guild: &NewGuild) -> impl Future<Output = Result<(), RepoError>> + Send;

    fn add_member(
        &self,
        realm_id: RealmId,
        user_id: UserId,
    ) -> impl Future<Output = Result<(), RepoError>> + Send;

    /// 권한 검사용: 유저가 Realm 멤버인가.
    fn is_member(
        &self,
        realm_id: RealmId,
        user_id: UserId,
    ) -> impl Future<Output = Result<bool, RepoError>> + Send;

    /// 자동 구독용(D13): 유저가 속한 Realm 목록.
    fn member_realm_ids(
        &self,
        user_id: UserId,
    ) -> impl Future<Output = Result<Vec<RealmId>, RepoError>> + Send;
}

/// 채널 저장소 port.
pub trait ChannelRepository: Send + Sync {
    fn create_channel(
        &self,
        channel: &NewChannel,
    ) -> impl Future<Output = Result<(), RepoError>> + Send;

    fn get(
        &self,
        id: ChannelId,
    ) -> impl Future<Output = Result<Option<Channel>, RepoError>> + Send;

    fn list_by_realm(
        &self,
        realm_id: RealmId,
    ) -> impl Future<Output = Result<Vec<Channel>, RepoError>> + Send;
}

/// 메시지 저장소 port (D24 persist / D34 nonce / D38 페이지네이션).
pub trait MessageRepository: Send + Sync {
    /// persist. nonce 중복이면 `Ok(false)`(삽입 안 됨, 멱등) — ON CONFLICT DO NOTHING.
    fn create_message(
        &self,
        msg: &NewMessage,
    ) -> impl Future<Output = Result<bool, RepoError>> + Send;

    /// 채널 히스토리 — `before`(Snowflake 커서) 이전 최신순 `limit`개 (D38).
    fn list_by_channel(
        &self,
        channel_id: ChannelId,
        before: Option<MessageId>,
        limit: i64,
    ) -> impl Future<Output = Result<Vec<Message>, RepoError>> + Send;
}

/// 모든 저장소 port를 한 타입이 구현 — 조합 루트에서 제네릭 1개로 주입 (제네릭 폭발 방지).
pub trait Store:
    UserRepository + RefreshTokenRepository + GuildRepository + ChannelRepository + MessageRepository
{
}

impl<T> Store for T where
    T: UserRepository + RefreshTokenRepository + GuildRepository + ChannelRepository + MessageRepository
{
}
