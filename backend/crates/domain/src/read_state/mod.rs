//! 읽음 상태 (개념: read_state). 순수 데이터 — IO 무의존. 스키마 02-schema.md §8.
//!
//! 채널별 "어디까지 읽었나"(`last_read_message_id`) + 안 읽은 멘션 수(`mention_count`).
//! Discord UX의 미읽음 배지/멘션 카운트 원천. 클라는 READY 스냅샷으로 받고 ack로 갱신한다.

use crate::id::{ChannelId, MessageId};

/// (user, channel) 한 행의 읽음 상태.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadState {
    pub channel_id: ChannelId,
    /// 마지막으로 읽은 메시지 id. 한 번도 안 읽었으면 None.
    pub last_read_message_id: Option<MessageId>,
    /// 안 읽은(= last_read 이후) 멘션 수.
    pub mention_count: i32,
}
