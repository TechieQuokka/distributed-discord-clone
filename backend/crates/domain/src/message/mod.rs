//! 메시지 엔티티 (개념: message). 순수 데이터 — IO 무의존. 스키마 02-schema.md §5.
//!
//! nonce는 클라 멱등성 키 (D34) — 같은 (channel, author, nonce) 재전송은 중복 무시.

use crate::id::{ChannelId, MessageId, RealmId, UserId};

/// 저장된 메시지.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub id: MessageId,
    pub channel_id: ChannelId,
    pub realm_id: RealmId,
    pub author_id: UserId,
    pub content: String,
    pub nonce: Option<String>,
}

/// 신규 메시지 생성 입력.
#[derive(Clone, Debug)]
pub struct NewMessage {
    pub id: MessageId,
    pub channel_id: ChannelId,
    pub realm_id: RealmId,
    pub author_id: UserId,
    pub content: String,
    pub nonce: Option<String>,
}
