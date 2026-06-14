//! 채널 엔티티 (개념: channel). 순수 데이터 — IO 무의존. 스키마 02-schema.md §4.

use crate::id::{ChannelId, RealmId};

/// 채널 종류 (DB `channel_kind` enum 대응).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelKind {
    Text,
    Voice,
    Category,
    Announcement,
    Forum,
    Thread,
    Dm,
}

impl ChannelKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ChannelKind::Text => "text",
            ChannelKind::Voice => "voice",
            ChannelKind::Category => "category",
            ChannelKind::Announcement => "announcement",
            ChannelKind::Forum => "forum",
            ChannelKind::Thread => "thread",
            ChannelKind::Dm => "dm",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "text" => ChannelKind::Text,
            "voice" => ChannelKind::Voice,
            "category" => ChannelKind::Category,
            "announcement" => ChannelKind::Announcement,
            "forum" => ChannelKind::Forum,
            "thread" => ChannelKind::Thread,
            "dm" => ChannelKind::Dm,
            _ => return None,
        })
    }
}

/// 저장된 채널.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Channel {
    pub id: ChannelId,
    pub realm_id: RealmId,
    pub kind: ChannelKind,
    pub name: Option<String>,
    pub position: i32,
}

/// 신규 채널 생성 입력.
#[derive(Clone, Debug)]
pub struct NewChannel {
    pub id: ChannelId,
    pub realm_id: RealmId,
    pub kind: ChannelKind,
    pub name: String,
}
