//! 웹훅 엔티티 (개념: webhook). 순수 데이터 — IO 무의존. 스키마 02-schema.md §8.
//!
//! 채널에 토큰으로 메시지를 게시하는 외부 진입점. 토큰 원본은 생성 시 1회 반환, 저장은 SHA-256 해시만(D14).

use crate::id::{ChannelId, RealmId, UserId, WebhookId};

/// 저장된 웹훅 (token_hash는 검증용 — 뷰엔 노출 안 함).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Webhook {
    pub id: WebhookId,
    pub channel_id: ChannelId,
    pub realm_id: RealmId,
    pub name: String,
    pub creator_id: Option<UserId>,
    /// SHA-256(원본 토큰). 실행 시 제시 토큰 해시와 비교.
    pub token_hash: Vec<u8>,
}

/// 신규 웹훅 입력.
#[derive(Clone, Debug)]
pub struct NewWebhook {
    pub id: WebhookId,
    pub channel_id: ChannelId,
    pub realm_id: RealmId,
    pub name: String,
    pub creator_id: UserId,
    pub token_hash: Vec<u8>,
}
