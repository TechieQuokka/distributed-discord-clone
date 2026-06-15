//! `ThreadRepository` 구현 for `PgStore` (개념: thread). adapter (D22).
//!
//! 스레드 생성 = channels(kind='thread', parent_id) + thread_meta(owner) 한 트랜잭션.
//! message_count는 조회 시 messages에서 집계(쓰기 경로 비결합).

use domain::id::{ChannelId, RealmId, Snowflake, UserId};
use domain::repo::{RepoError, ThreadRepository};
use domain::thread::{NewThread, Thread};
use sqlx::Row;

use crate::store::{PgStore, map_err};

fn row_to_thread(r: &sqlx::postgres::PgRow) -> Thread {
    let id: i64 = r.get("id");
    let realm_id: i64 = r.get("realm_id");
    let parent_id: i64 = r.get("parent_id");
    let owner: Option<i64> = r.get("owner_id");
    Thread {
        id: ChannelId(Snowflake::from_raw(id as u64)),
        realm_id: RealmId(Snowflake::from_raw(realm_id as u64)),
        parent_id: ChannelId(Snowflake::from_raw(parent_id as u64)),
        name: r.get("name"),
        owner_id: owner.map(|o| UserId(Snowflake::from_raw(o as u64))),
        archived: r.get("archived"),
        auto_archive: r.get("auto_archive"),
        message_count: r.get("message_count"),
    }
}

/// SELECT 본문(채널+메타 조인 + 살아있는 메시지 집계). 리터럴 쿼리로 두 가지(get/list) 유지
/// — sqlx 0.9는 동적 SQL 문자열을 거부(injection 가드)하므로 format! 대신 const 분리.
const SELECT_THREAD_BY_ID: &str = "SELECT c.id, c.realm_id, c.parent_id, c.name, \
     tm.owner_id, tm.archived, tm.auto_archive, \
     (SELECT count(*) FROM messages m WHERE m.channel_id = c.id AND m.deleted_at IS NULL) AS message_count \
     FROM channels c JOIN thread_meta tm ON tm.channel_id = c.id \
     WHERE c.deleted_at IS NULL AND c.id = $1";
const SELECT_THREADS_BY_PARENT: &str = "SELECT c.id, c.realm_id, c.parent_id, c.name, \
     tm.owner_id, tm.archived, tm.auto_archive, \
     (SELECT count(*) FROM messages m WHERE m.channel_id = c.id AND m.deleted_at IS NULL) AS message_count \
     FROM channels c JOIN thread_meta tm ON tm.channel_id = c.id \
     WHERE c.deleted_at IS NULL AND c.parent_id = $1 ORDER BY c.id DESC";

impl ThreadRepository for PgStore {
    async fn create_thread(&self, t: &NewThread) -> Result<(), RepoError> {
        let mut tx = self.pool.begin().await.map_err(map_err)?;
        sqlx::query(
            "INSERT INTO channels (id, realm_id, kind, name, parent_id) \
             VALUES ($1, $2, 'thread'::channel_kind, $3, $4)",
        )
        .bind(t.id.0.raw() as i64)
        .bind(t.realm_id.0.raw() as i64)
        .bind(&t.name)
        .bind(t.parent_id.0.raw() as i64)
        .execute(&mut *tx)
        .await
        .map_err(map_err)?;
        sqlx::query(
            "INSERT INTO thread_meta (channel_id, owner_id, auto_archive) VALUES ($1, $2, $3)",
        )
        .bind(t.id.0.raw() as i64)
        .bind(t.owner.0.raw() as i64)
        .bind(t.auto_archive)
        .execute(&mut *tx)
        .await
        .map_err(map_err)?;
        tx.commit().await.map_err(map_err)?;
        Ok(())
    }

    async fn get_thread(&self, channel: ChannelId) -> Result<Option<Thread>, RepoError> {
        let row = sqlx::query(SELECT_THREAD_BY_ID)
            .bind(channel.0.raw() as i64)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(row.as_ref().map(row_to_thread))
    }

    async fn list_threads(&self, parent: ChannelId) -> Result<Vec<Thread>, RepoError> {
        let rows = sqlx::query(SELECT_THREADS_BY_PARENT)
            .bind(parent.0.raw() as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(rows.iter().map(row_to_thread).collect())
    }

    async fn set_thread_archived(
        &self,
        channel: ChannelId,
        archived: bool,
    ) -> Result<bool, RepoError> {
        let res = sqlx::query("UPDATE thread_meta SET archived = $2 WHERE channel_id = $1")
            .bind(channel.0.raw() as i64)
            .bind(archived)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(res.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PgStore, connect, run_migrations};
    use domain::channel::{ChannelKind, NewChannel};
    use domain::guild::NewGuild;
    use domain::id::MessageId;
    use domain::message::NewMessage;
    use domain::repo::{ChannelRepository, GuildRepository, MessageRepository, UserRepository};
    use domain::user::NewUser;

    /// 실제 Postgres 필요 — skip if no DATABASE_URL. 스레드 생성/조회/목록/아카이브 + 메시지 집계.
    #[tokio::test]
    async fn thread_lifecycle() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL 미설정 — thread 테스트 skip");
            return;
        };
        let pool = connect(&url).await.expect("connect");
        run_migrations(&pool).await.expect("migrate");
        let s = PgStore::new(pool.clone());

        let owner = UserId(Snowflake::from_raw(740_001));
        let realm = RealmId(Snowflake::from_raw(740_002));
        let parent = ChannelId(Snowflake::from_raw(740_003));
        let thread = ChannelId(Snowflake::from_raw(740_004));
        sqlx::query("DELETE FROM messages WHERE channel_id = ANY($1)")
            .bind(vec![parent.0.raw() as i64, thread.0.raw() as i64]).execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM realms WHERE id = $1").bind(realm.0.raw() as i64).execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM users WHERE id = $1").bind(owner.0.raw() as i64).execute(&pool).await.unwrap();

        s.create_user(&NewUser { id: owner, username: "th_owner".into(), email: "th@e.com".into(), password_hash: "x".into() }).await.unwrap();
        s.create_guild(&NewGuild { realm_id: realm, name: "G".into(), owner_id: owner }).await.unwrap();
        s.create_channel(&NewChannel { id: parent, realm_id: realm, kind: ChannelKind::Text, name: "general".into() }).await.unwrap();

        // 스레드 생성 + 조회.
        s.create_thread(&NewThread { id: thread, realm_id: realm, parent_id: parent, name: "discussion".into(), owner, auto_archive: 1440 }).await.unwrap();
        let t = s.get_thread(thread).await.unwrap().expect("thread exists");
        assert_eq!(t.parent_id, parent);
        assert_eq!(t.owner_id, Some(owner));
        assert!(!t.archived);
        assert_eq!(t.message_count, 0);

        // 스레드는 일반 채널로도 보임 (kind=thread, parent_id 설정).
        let as_chan = s.get(thread).await.unwrap().unwrap();
        assert_eq!(as_chan.kind, ChannelKind::Thread);

        // 스레드에 메시지 → message_count 집계 반영.
        s.create_message(&NewMessage { id: MessageId(Snowflake::from_raw(740_010)), channel_id: thread, realm_id: realm, author_id: owner, content: "hi thread".into(), nonce: None, reference_message_id: None }).await.unwrap();
        assert_eq!(s.get_thread(thread).await.unwrap().unwrap().message_count, 1);

        // 부모별 목록.
        let list = s.list_threads(parent).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, thread);

        // 아카이브.
        assert!(s.set_thread_archived(thread, true).await.unwrap());
        assert!(s.get_thread(thread).await.unwrap().unwrap().archived);
        // 비-스레드 채널 아카이브는 false.
        assert!(!s.set_thread_archived(parent, true).await.unwrap());

        sqlx::query("DELETE FROM messages WHERE channel_id = ANY($1)")
            .bind(vec![parent.0.raw() as i64, thread.0.raw() as i64]).execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM realms WHERE id = $1").bind(realm.0.raw() as i64).execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM users WHERE id = $1").bind(owner.0.raw() as i64).execute(&pool).await.unwrap();
    }
}
