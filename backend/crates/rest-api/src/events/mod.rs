//! 멤버 이벤트 JSON 페이로드 빌더 (개념: events). D39 — REST가 생산 엣지로 1회 직렬화.
//!
//! gateway.md §3 GUILD_MEMBER_* 스키마에 대응. 결과 문자열은 `RealmEmitter::emit`에 그대로 실린다
//! (하위 계층은 불투명 통과, 최종 배달 직전 gateway가 역파싱).

use domain::id::{ChannelId, MessageId, RealmId, UserId};
use domain::member::Member;
use domain::message::Message;
use domain::relationship::RelationKind;
use domain::user::User;
use serde_json::json;

/// `GUILD_MEMBER_ADD` / `GUILD_MEMBER_UPDATE` 페이로드. `member`로 nick/roles 채움.
pub fn member_upsert_payload(realm: RealmId, user: &User, member: Option<&Member>) -> String {
    json!({
        "realm_id": realm.0.raw().to_string(),
        "user": { "id": user.id.0.raw().to_string(), "username": user.username },
        "nick": member.and_then(|m| m.nick.clone()),
        "roles": member
            .map(|m| m.roles.iter().map(|r| r.0.raw().to_string()).collect::<Vec<_>>())
            .unwrap_or_default(),
    })
    .to_string()
}

/// `GUILD_MEMBER_REMOVE` 페이로드 (user id만).
pub fn member_remove_payload(realm: RealmId, user_id: UserId) -> String {
    json!({
        "realm_id": realm.0.raw().to_string(),
        "user": { "id": user_id.0.raw().to_string() },
    })
    .to_string()
}

/// `RELATIONSHIP_ADD` 페이로드. 수신자 관점의 상대(`user`) + 관계 종류(`kind`).
pub fn relationship_add_payload(other: &User, kind: RelationKind) -> String {
    json!({
        "user": { "id": other.id.0.raw().to_string(), "username": other.username },
        "kind": kind.as_str(),
    })
    .to_string()
}

/// `RELATIONSHIP_REMOVE` 페이로드 (상대 user id만).
pub fn relationship_remove_payload(other: UserId) -> String {
    json!({ "user": { "id": other.0.raw().to_string() } }).to_string()
}

/// `MESSAGE_ACK` 페이로드 (읽음 처리). 유저 다른 기기 동기화용.
pub fn message_ack_payload(channel_id: ChannelId, message_id: MessageId, mention_count: i32) -> String {
    json!({
        "channel_id": channel_id.0.raw().to_string(),
        "message_id": message_id.0.raw().to_string(),
        "mention_count": mention_count,
    })
    .to_string()
}

/// `MESSAGE_UPDATE` 페이로드 (편집된 메시지). `content`는 새 내용.
pub fn message_update_payload(msg: &Message, content: &str) -> String {
    json!({
        "id": msg.id.0.raw().to_string(),
        "channel_id": msg.channel_id.0.raw().to_string(),
        "author": { "id": msg.author_id.0.raw().to_string() },
        "content": content,
        "edited": true,
    })
    .to_string()
}

/// `MESSAGE_DELETE` 페이로드 (id + channel만).
pub fn message_delete_payload(channel_id: ChannelId, message_id: MessageId) -> String {
    json!({
        "id": message_id.0.raw().to_string(),
        "channel_id": channel_id.0.raw().to_string(),
    })
    .to_string()
}

/// `CHANNEL_RECIPIENT_ADD` 페이로드 (그룹DM 참가자 추가, D8). user + channel.
pub fn recipient_add_payload(realm: RealmId, channel_id: ChannelId, user: &User) -> String {
    json!({
        "realm_id": realm.0.raw().to_string(),
        "channel_id": channel_id.0.raw().to_string(),
        "user": { "id": user.id.0.raw().to_string(), "username": user.username },
    })
    .to_string()
}

/// `CHANNEL_RECIPIENT_REMOVE` 페이로드 (그룹DM 참가자 제거/탈퇴). user id만.
pub fn recipient_remove_payload(realm: RealmId, channel_id: ChannelId, user_id: UserId) -> String {
    json!({
        "realm_id": realm.0.raw().to_string(),
        "channel_id": channel_id.0.raw().to_string(),
        "user": { "id": user_id.0.raw().to_string() },
    })
    .to_string()
}

/// `MESSAGE_REACTION_ADD` / `_REMOVE` 페이로드.
pub fn reaction_payload(
    channel_id: ChannelId,
    message_id: MessageId,
    user: UserId,
    emoji: &str,
) -> String {
    json!({
        "message_id": message_id.0.raw().to_string(),
        "channel_id": channel_id.0.raw().to_string(),
        "user_id": user.0.raw().to_string(),
        "emoji": emoji,
    })
    .to_string()
}
