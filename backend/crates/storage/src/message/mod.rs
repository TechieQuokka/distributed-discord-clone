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
    Message {
        id: MessageId(Snowflake::from_raw(id as u64)),
        channel_id: ChannelId(Snowflake::from_raw(channel_id as u64)),
        realm_id: RealmId(Snowflake::from_raw(realm_id as u64)),
        author_id: UserId(Snowflake::from_raw(author_id as u64)),
        content: r.get("content"),
        nonce: r.get("nonce"),
    }
}

impl MessageRepository for PgStore {
    async fn create_message(&self, m: &NewMessage) -> Result<bool, RepoError> {
        // nonce 있을 때만 충돌 가능(부분 유니크). RETURNING id 행이 있으면 신규 삽입.
        let q = if m.nonce.is_some() {
            "INSERT INTO messages (id, channel_id, realm_id, author_id, content, nonce) \
             VALUES ($1, $2, $3, $4, $5, $6) \
             ON CONFLICT (channel_id, author_id, nonce) WHERE nonce IS NOT NULL DO NOTHING RETURNING id"
        } else {
            "INSERT INTO messages (id, channel_id, realm_id, author_id, content, nonce) \
             VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"
        };
        let inserted = sqlx::query(q)
            .bind(m.id.0.raw() as i64)
            .bind(m.channel_id.0.raw() as i64)
            .bind(m.realm_id.0.raw() as i64)
            .bind(m.author_id.0.raw() as i64)
            .bind(&m.content)
            .bind(&m.nonce)
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
            "SELECT id, channel_id, realm_id, author_id, content, nonce FROM messages \
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
        assert!(s.create_message(&NewMessage { id: m1, channel_id: chan, realm_id: realm, author_id: owner, content: "first".into(), nonce: Some("n1".into()) }).await.unwrap());
        // 같은 nonce 재전송 → 중복(false).
        assert!(!s.create_message(&NewMessage { id: MessageId(Snowflake::from_raw(700_099)), channel_id: chan, realm_id: realm, author_id: owner, content: "dup".into(), nonce: Some("n1".into()) }).await.unwrap());
        assert!(s.create_message(&NewMessage { id: m2, channel_id: chan, realm_id: realm, author_id: owner, content: "second".into(), nonce: None }).await.unwrap());

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
