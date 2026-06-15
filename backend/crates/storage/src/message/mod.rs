//! `MessageRepository` 구현 for `PgStore` (개념: message). adapter (D24/D34/D38).
//!
//! - persist-then-fanout의 persist (D24).
//! - nonce 멱등성(D34): `uq_messages_nonce(channel_id, author_id, nonce)` 부분 유니크 →
//!   nonce 있으면 ON CONFLICT DO NOTHING, RETURNING 유무로 신규/중복 판정.
//! - 히스토리(D38): `before` Snowflake 커서 기준 `id DESC` 페이지.

use domain::id::{ChannelId, MessageId, RealmId, Snowflake, UserId};
use domain::message::{Message, NewMessage};
use domain::repo::{MessageRepository, RepoError};
use sqlx::Row;

use crate::store::{PgStore, map_err};

fn row_to_message(r: &sqlx::postgres::PgRow) -> Message {
    let id: i64 = r.get("id");
    let channel_id: i64 = r.get("channel_id");
    let realm_id: i64 = r.get("realm_id");
    let author_id: i64 = r.get("author_id");
    let reference: Option<i64> = r.get("reference_message_id");
    Message {
        id: MessageId(Snowflake::from_raw(id as u64)),
        channel_id: ChannelId(Snowflake::from_raw(channel_id as u64)),
        realm_id: RealmId(Snowflake::from_raw(realm_id as u64)),
        author_id: UserId(Snowflake::from_raw(author_id as u64)),
        content: r.get("content"),
        nonce: r.get("nonce"),
        reference_message_id: reference.map(|n| MessageId(Snowflake::from_raw(n as u64))),
    }
}

impl MessageRepository for PgStore {
    async fn create_message(&self, m: &NewMessage) -> Result<bool, RepoError> {
        // nonce 있을 때만 충돌 가능(부분 유니크). RETURNING id 행이 있으면 신규 삽입.
        let q = if m.nonce.is_some() {
            "INSERT INTO messages (id, channel_id, realm_id, author_id, content, nonce, reference_message_id) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT (channel_id, author_id, nonce) WHERE nonce IS NOT NULL DO NOTHING RETURNING id"
        } else {
            "INSERT INTO messages (id, channel_id, realm_id, author_id, content, nonce, reference_message_id) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id"
        };
        let inserted = sqlx::query(q)
            .bind(m.id.0.raw() as i64)
            .bind(m.channel_id.0.raw() as i64)
            .bind(m.realm_id.0.raw() as i64)
            .bind(m.author_id.0.raw() as i64)
            .bind(&m.content)
            .bind(&m.nonce)
            .bind(m.reference_message_id.map(|r| r.0.raw() as i64))
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(inserted.is_some())
    }

    async fn list_by_channel(
        &self,
        channel_id: ChannelId,
        before: Option<MessageId>,
        limit: i64,
    ) -> Result<Vec<Message>, RepoError> {
        let rows = sqlx::query(
            "SELECT id, channel_id, realm_id, author_id, content, nonce, reference_message_id FROM messages \
             WHERE channel_id = $1 AND deleted_at IS NULL \
               AND ($2::bigint IS NULL OR id < $2) \
             ORDER BY id DESC LIMIT $3",
        )
        .bind(channel_id.0.raw() as i64)
        .bind(before.map(|b| b.0.raw() as i64))
        .bind(limit.clamp(1, 100))
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(rows.iter().map(row_to_message).collect())
    }

    async fn get_message(&self, id: MessageId) -> Result<Option<Message>, RepoError> {
        let row = sqlx::query(
            "SELECT id, channel_id, realm_id, author_id, content, nonce, reference_message_id FROM messages \
             WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(id.0.raw() as i64)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(row.as_ref().map(row_to_message))
    }

    async fn edit_message(
        &self,
        id: MessageId,
        author: UserId,
        content: &str,
    ) -> Result<bool, RepoError> {
        // 작성자 본인 + 미삭제만 수정 (신뢰 경계는 SQL 조건으로도 한 번 더 보강).
        let res = sqlx::query(
            "UPDATE messages SET content = $3, edited_at = now() \
             WHERE id = $1 AND author_id = $2 AND deleted_at IS NULL",
        )
        .bind(id.0.raw() as i64)
        .bind(author.0.raw() as i64)
        .bind(content)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(res.rows_affected() > 0)
    }

    async fn soft_delete_message(&self, id: MessageId) -> Result<bool, RepoError> {
        let res = sqlx::query(
            "UPDATE messages SET deleted_at = now() WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(id.0.raw() as i64)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(res.rows_affected() > 0)
    }

    async fn add_mentions(&self, message_id: MessageId, users: &[UserId]) -> Result<(), RepoError> {
        if users.is_empty() {
            return Ok(());
        }
        let ids: Vec<i64> = users.iter().map(|u| u.0.raw() as i64).collect();
        // 존재하는 유저만 적재(FK 위반 방지) + 멱등. UNNEST로 한 번에.
        sqlx::query(
            "INSERT INTO message_mentions (message_id, user_id) \
             SELECT $1, u FROM unnest($2::bigint[]) AS u \
             WHERE EXISTS (SELECT 1 FROM users WHERE id = u) \
             ON CONFLICT DO NOTHING",
        )
        .bind(message_id.0.raw() as i64)
        .bind(&ids)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::channel::{ChannelKind, NewChannel};
    use domain::guild::NewGuild;
    use domain::repo::{ChannelRepository, GuildRepository, UserRepository};
    use domain::user::NewUser;

    /// 실제 Postgres 필요 — `DATABASE_URL` 미설정 시 skip.
    /// 길드→채널→메시지 persist + nonce 멱등 + 페이지네이션 종단 검증.
    #[tokio::test]
    async fn persist_nonce_idempotency_and_pagination() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL 미설정 — message 테스트 skip");
            return;
        };
        let pool = crate::connect(&url).await.expect("connect");
        crate::run_migrations(&pool).await.expect("migrate");
        let s = PgStore::new(pool.clone());

        let owner = UserId(Snowflake::from_raw(700_001));
        let realm = RealmId(Snowflake::from_raw(700_002));
        let chan = ChannelId(Snowflake::from_raw(700_003));
        // 정리 (CASCADE: realm 삭제 시 channels/members/guild 삭제, messages는 FK 없음 → 수동).
        sqlx::query("DELETE FROM messages WHERE channel_id = $1").bind(chan.0.raw() as i64).execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM realms WHERE id = $1").bind(realm.0.raw() as i64).execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM users WHERE id = $1").bind(owner.0.raw() as i64).execute(&pool).await.unwrap();

        s.create_user(&NewUser { id: owner, username: "msg_owner".into(), email: "msg@e.com".into(), password_hash: "x".into() }).await.unwrap();
        s.create_guild(&NewGuild { realm_id: realm, name: "G".into(), owner_id: owner }).await.unwrap();
        s.create_channel(&NewChannel { id: chan, realm_id: realm, kind: ChannelKind::Text, name: "general".into() }).await.unwrap();

        // 메시지 2개.
        let m1 = MessageId(Snowflake::from_raw(700_010));
        let m2 = MessageId(Snowflake::from_raw(700_011));
        assert!(s.create_message(&NewMessage { id: m1, channel_id: chan, realm_id: realm, author_id: owner, content: "first".into(), nonce: Some("n1".into()), reference_message_id: None }).await.unwrap());
        // 같은 nonce 재전송 → 중복(false).
        assert!(!s.create_message(&NewMessage { id: MessageId(Snowflake::from_raw(700_099)), channel_id: chan, realm_id: realm, author_id: owner, content: "dup".into(), nonce: Some("n1".into()), reference_message_id: None }).await.unwrap());
        assert!(s.create_message(&NewMessage { id: m2, channel_id: chan, realm_id: realm, author_id: owner, content: "second".into(), nonce: None, reference_message_id: None }).await.unwrap());

        // 페이지네이션: 최신순.
        let page = s.list_by_channel(chan, None, 10).await.unwrap();
        assert_eq!(page.len(), 2);
        assert_eq!(page[0].id, m2, "최신이 먼저");
        assert_eq!(page[1].id, m1);
        // before 커서.
        let older = s.list_by_channel(chan, Some(m2), 10).await.unwrap();
        assert_eq!(older.len(), 1);
        assert_eq!(older[0].id, m1);

        sqlx::query("DELETE FROM messages WHERE channel_id = $1").bind(chan.0.raw() as i64).execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM realms WHERE id = $1").bind(realm.0.raw() as i64).execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM users WHERE id = $1").bind(owner.0.raw() as i64).execute(&pool).await.unwrap();
    }
}

#[cfg(test)]
mod edit_react_tests {
    use super::*;
    use domain::channel::{ChannelKind, NewChannel};
    use domain::guild::NewGuild;
    use domain::repo::{ChannelRepository, GuildRepository, ReactionRepository, UserRepository};
    use domain::message::NewMessage;
    use domain::user::NewUser;

    /// 실제 Postgres 필요 — skip if no DATABASE_URL. 편집/소프트삭제/리액션 종단.
    #[tokio::test]
    async fn edit_delete_reaction_lifecycle() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL 미설정 — edit/react 테스트 skip");
            return;
        };
        let pool = crate::connect(&url).await.expect("connect");
        crate::run_migrations(&pool).await.expect("migrate");
        let s = PgStore::new(pool.clone());

        let owner = UserId(Snowflake::from_raw(710_001));
        let other = UserId(Snowflake::from_raw(710_004));
        let realm = RealmId(Snowflake::from_raw(710_002));
        let chan = ChannelId(Snowflake::from_raw(710_003));
        let mid = MessageId(Snowflake::from_raw(710_010));
        sqlx::query("DELETE FROM messages WHERE channel_id = $1").bind(chan.0.raw() as i64).execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM realms WHERE id = $1").bind(realm.0.raw() as i64).execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM users WHERE id = ANY($1)").bind(vec![owner.0.raw() as i64, other.0.raw() as i64]).execute(&pool).await.unwrap();

        s.create_user(&NewUser { id: owner, username: "er_owner".into(), email: "er_o@e.com".into(), password_hash: "x".into() }).await.unwrap();
        s.create_user(&NewUser { id: other, username: "er_other".into(), email: "er_x@e.com".into(), password_hash: "x".into() }).await.unwrap();
        s.create_guild(&NewGuild { realm_id: realm, name: "G".into(), owner_id: owner }).await.unwrap();
        s.create_channel(&NewChannel { id: chan, realm_id: realm, kind: ChannelKind::Text, name: "general".into() }).await.unwrap();
        s.create_message(&NewMessage { id: mid, channel_id: chan, realm_id: realm, author_id: owner, content: "orig".into(), nonce: None, reference_message_id: None }).await.unwrap();

        // 편집: 작성자만. 타인 시도 → false, 작성자 → true + 내용 반영.
        assert!(!s.edit_message(mid, other, "hax").await.unwrap(), "비작성자 편집 불가");
        assert!(s.edit_message(mid, owner, "edited").await.unwrap());
        assert_eq!(s.get_message(mid).await.unwrap().unwrap().content, "edited");

        // 리액션: 추가 멱등 + 제거.
        assert!(s.add_reaction(mid, other, "👍").await.unwrap());
        assert!(!s.add_reaction(mid, other, "👍").await.unwrap(), "중복 추가는 false(멱등)");
        assert!(s.remove_reaction(mid, other, "👍").await.unwrap());
        assert!(!s.remove_reaction(mid, other, "👍").await.unwrap(), "없는 리액션 제거는 false");

        // 소프트 삭제: get/list에서 사라지고, 재삭제는 false.
        assert!(s.soft_delete_message(mid).await.unwrap());
        assert!(s.get_message(mid).await.unwrap().is_none(), "삭제 후 조회 None");
        assert_eq!(s.list_by_channel(chan, None, 10).await.unwrap().len(), 0, "히스토리에서 제외");
        assert!(!s.soft_delete_message(mid).await.unwrap(), "이미 삭제 → false");
        assert!(!s.edit_message(mid, owner, "zzz").await.unwrap(), "삭제된 메시지 편집 불가");

        sqlx::query("DELETE FROM messages WHERE channel_id = $1").bind(chan.0.raw() as i64).execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM realms WHERE id = $1").bind(realm.0.raw() as i64).execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM users WHERE id = ANY($1)").bind(vec![owner.0.raw() as i64, other.0.raw() as i64]).execute(&pool).await.unwrap();
    }
}

#[cfg(test)]
mod reply_mention_tests {
    use super::*;
    use domain::channel::{ChannelKind, NewChannel};
    use domain::guild::NewGuild;
    use domain::repo::{ChannelRepository, GuildRepository, UserRepository};
    use domain::user::NewUser;

    /// 실제 Postgres 필요 — skip if no DATABASE_URL. 답장(reference) + 멘션(존재 유저만) 종단.
    #[tokio::test]
    async fn reply_reference_and_mentions() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL 미설정 — reply/mention 테스트 skip");
            return;
        };
        let pool = crate::connect(&url).await.expect("connect");
        crate::run_migrations(&pool).await.expect("migrate");
        let s = PgStore::new(pool.clone());

        let owner = UserId(Snowflake::from_raw(720_001));
        let mentioned = UserId(Snowflake::from_raw(720_004));
        let realm = RealmId(Snowflake::from_raw(720_002));
        let chan = ChannelId(Snowflake::from_raw(720_003));
        let m1 = MessageId(Snowflake::from_raw(720_010));
        let m2 = MessageId(Snowflake::from_raw(720_011));
        sqlx::query("DELETE FROM messages WHERE channel_id = $1").bind(chan.0.raw() as i64).execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM realms WHERE id = $1").bind(realm.0.raw() as i64).execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM users WHERE id = ANY($1)").bind(vec![owner.0.raw() as i64, mentioned.0.raw() as i64]).execute(&pool).await.unwrap();

        s.create_user(&NewUser { id: owner, username: "rm_owner".into(), email: "rm_o@e.com".into(), password_hash: "x".into() }).await.unwrap();
        s.create_user(&NewUser { id: mentioned, username: "rm_ment".into(), email: "rm_m@e.com".into(), password_hash: "x".into() }).await.unwrap();
        s.create_guild(&NewGuild { realm_id: realm, name: "G".into(), owner_id: owner }).await.unwrap();
        s.create_channel(&NewChannel { id: chan, realm_id: realm, kind: ChannelKind::Text, name: "general".into() }).await.unwrap();

        // m1 원본, m2 = m1 답장.
        s.create_message(&NewMessage { id: m1, channel_id: chan, realm_id: realm, author_id: owner, content: "orig".into(), nonce: None, reference_message_id: None }).await.unwrap();
        s.create_message(&NewMessage { id: m2, channel_id: chan, realm_id: realm, author_id: owner, content: "reply".into(), nonce: None, reference_message_id: Some(m1) }).await.unwrap();
        assert_eq!(s.get_message(m2).await.unwrap().unwrap().reference_message_id, Some(m1));
        assert_eq!(s.get_message(m1).await.unwrap().unwrap().reference_message_id, None);

        // 멘션: 존재 유저(mentioned) + 미존재(99999) → 존재하는 것만 적재.
        let ghost = UserId(Snowflake::from_raw(720_099));
        s.add_mentions(m2, &[mentioned, ghost]).await.unwrap();
        let cnt: i64 = sqlx::query_scalar("SELECT count(*) FROM message_mentions WHERE message_id = $1")
            .bind(m2.0.raw() as i64).fetch_one(&pool).await.unwrap();
        assert_eq!(cnt, 1, "존재 유저만 적재(ghost 제외)");
        // 멱등 재적재.
        s.add_mentions(m2, &[mentioned]).await.unwrap();
        let cnt2: i64 = sqlx::query_scalar("SELECT count(*) FROM message_mentions WHERE message_id = $1")
            .bind(m2.0.raw() as i64).fetch_one(&pool).await.unwrap();
        assert_eq!(cnt2, 1, "멱등");

        sqlx::query("DELETE FROM messages WHERE channel_id = $1").bind(chan.0.raw() as i64).execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM realms WHERE id = $1").bind(realm.0.raw() as i64).execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM users WHERE id = ANY($1)").bind(vec![owner.0.raw() as i64, mentioned.0.raw() as i64]).execute(&pool).await.unwrap();
    }
}
