//! `AttachmentRepository` 구현 for `PgStore` (개념: attachment). adapter (D22/D37).
//! 메타데이터만 — 바이트는 [`crate::blob::LocalFsBlobStore`].

use domain::attachment::{Attachment, NewAttachment};
use domain::id::{AttachmentId, MessageId, Snowflake};
use domain::repo::{AttachmentRepository, RepoError};
use sqlx::Row;

use crate::store::{PgStore, map_err};

fn row_to_attachment(r: &sqlx::postgres::PgRow) -> Attachment {
    let id: i64 = r.get("id");
    let message_id: i64 = r.get("message_id");
    Attachment {
        id: AttachmentId(Snowflake::from_raw(id as u64)),
        message_id: MessageId(Snowflake::from_raw(message_id as u64)),
        filename: r.get("filename"),
        size_bytes: r.get("size_bytes"),
        content_type: r.get("content_type"),
        url: r.get("url"),
    }
}

impl AttachmentRepository for PgStore {
    async fn add_attachment(&self, a: &NewAttachment) -> Result<(), RepoError> {
        sqlx::query(
            "INSERT INTO attachments (id, message_id, filename, size_bytes, content_type, url) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(a.id.0.raw() as i64)
        .bind(a.message_id.0.raw() as i64)
        .bind(&a.filename)
        .bind(a.size_bytes)
        .bind(&a.content_type)
        .bind(&a.url)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(())
    }

    async fn list_attachments(&self, message_id: MessageId) -> Result<Vec<Attachment>, RepoError> {
        let rows = sqlx::query(
            "SELECT id, message_id, filename, size_bytes, content_type, url FROM attachments \
             WHERE message_id = $1 ORDER BY id",
        )
        .bind(message_id.0.raw() as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(rows.iter().map(row_to_attachment).collect())
    }

    async fn get_attachment(&self, id: AttachmentId) -> Result<Option<Attachment>, RepoError> {
        let row = sqlx::query(
            "SELECT id, message_id, filename, size_bytes, content_type, url FROM attachments WHERE id = $1",
        )
        .bind(id.0.raw() as i64)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(row.as_ref().map(row_to_attachment))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PgStore, connect, run_migrations};
    use domain::channel::{ChannelKind, NewChannel};
    use domain::guild::NewGuild;
    use domain::id::{ChannelId, RealmId, UserId};
    use domain::message::NewMessage;
    use domain::repo::{ChannelRepository, GuildRepository, MessageRepository, UserRepository};
    use domain::user::NewUser;

    /// 실제 Postgres 필요 — skip if no DATABASE_URL. 첨부 메타 적재/목록/조회 + 메시지 삭제 CASCADE.
    #[tokio::test]
    async fn attachment_metadata_lifecycle() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL 미설정 — attachment 테스트 skip");
            return;
        };
        let pool = connect(&url).await.expect("connect");
        run_migrations(&pool).await.expect("migrate");
        let s = PgStore::new(pool.clone());

        let owner = UserId(Snowflake::from_raw(750_001));
        let realm = RealmId(Snowflake::from_raw(750_002));
        let chan = ChannelId(Snowflake::from_raw(750_003));
        let mid = MessageId(Snowflake::from_raw(750_010));
        sqlx::query("DELETE FROM messages WHERE channel_id = $1").bind(chan.0.raw() as i64).execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM realms WHERE id = $1").bind(realm.0.raw() as i64).execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM users WHERE id = $1").bind(owner.0.raw() as i64).execute(&pool).await.unwrap();

        s.create_user(&NewUser { id: owner, username: "at_owner".into(), email: "at@e.com".into(), password_hash: "x".into() }).await.unwrap();
        s.create_guild(&NewGuild { realm_id: realm, name: "G".into(), owner_id: owner }).await.unwrap();
        s.create_channel(&NewChannel { id: chan, realm_id: realm, kind: ChannelKind::Text, name: "general".into() }).await.unwrap();
        s.create_message(&NewMessage { id: mid, channel_id: chan, realm_id: realm, author_id: owner, content: "see attachment".into(), nonce: None, reference_message_id: None }).await.unwrap();

        let aid = AttachmentId(Snowflake::from_raw(750_020));
        s.add_attachment(&NewAttachment {
            id: aid, message_id: mid, filename: "hello.txt".into(), size_bytes: 11,
            content_type: Some("text/plain".into()), url: format!("/attachments/{}", aid.0.raw()),
        }).await.unwrap();

        let list = s.list_attachments(mid).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].filename, "hello.txt");
        assert_eq!(list[0].size_bytes, 11);
        assert_eq!(s.get_attachment(aid).await.unwrap().unwrap().message_id, mid);

        // 메시지 하드 삭제 시 attachments CASCADE.
        sqlx::query("DELETE FROM messages WHERE id = $1").bind(mid.0.raw() as i64).execute(&pool).await.unwrap();
        assert!(s.get_attachment(aid).await.unwrap().is_none(), "메시지 삭제 → 첨부 CASCADE");

        sqlx::query("DELETE FROM realms WHERE id = $1").bind(realm.0.raw() as i64).execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM users WHERE id = $1").bind(owner.0.raw() as i64).execute(&pool).await.unwrap();
    }
}
