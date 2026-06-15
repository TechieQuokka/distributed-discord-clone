//! 멤버 이벤트 JSON 페이로드 빌더 (개념: events). D39 — REST가 생산 엣지로 1회 직렬화.
//!
//! gateway.md §3 GUILD_MEMBER_* 스키마에 대응. 결과 문자열은 `RealmEmitter::emit`에 그대로 실린다
//! (하위 계층은 불투명 통과, 최종 배달 직전 gateway가 역파싱).

use domain::id::{ChannelId, MessageId, RealmId, UserId};
use domain::member::Member;
use domain::message::Message;
use domain::relationship::RelationKind;
use domain::thread::Thread;
use domain::user::User;
use serde_json::json;

/// `THREAD_CREATE` / `THREAD_UPDATE` 페이로드 (스레드 = 채널, 부모/소유자/아카이브 포함).
pub fn thread_payload(t: &Thread) -> String {
    json!({
        "id": t.id.0.raw().to_string(),
        "realm_id": t.realm_id.0.raw().to_string(),
        "parent_id": t.parent_id.0.raw().to_string(),
        "name": t.name,
        "owner_id": t.owner_id.map(|o| o.0.raw().to_string()),
        "archived": t.archived,
        "auto_archive": t.auto_archive,
        "message_count": t.message_count,
    })
    .to_string()
}

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

/// 웹훅이 게시한 `MESSAGE_CREATE` 페이로드 (gateway dispatch의 메시지 페이로드 모양과 동일 + `webhook_id`).
/// rest-api가 persist 후 직접 emit하므로(액터 우회 seam) 여기서 1회 조립한다(D39 생산 엣지).
pub fn webhook_message_payload(
    message_id: MessageId,
    channel_id: ChannelId,
    author: UserId,
    webhook_id: u64,
    content: &str,
) -> String {
    json!({
        "id": message_id.0.raw().to_string(),
        "channel_id": channel_id.0.raw().to_string(),
        "author": { "id": author.0.raw().to_string() },
        "content": content,
        "nonce": serde_json::Value::Null,
        "reference_message_id": serde_json::Value::Null,
        "mentions": [],
        "webhook_id": webhook_id.to_string(),
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
