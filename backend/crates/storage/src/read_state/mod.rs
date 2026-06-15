//! `ReadStateRepository` 구현 for `PgStore` (개념: read_state). adapter (D22).
//!
//! ack는 last_read upsert + 그 이후 멘션 수 재계산을 한 문장으로. bump는 멘션 시 +1 upsert.

use domain::id::{ChannelId, MessageId, Snowflake, UserId};
use domain::read_state::ReadState;
use domain::repo::{ReadStateRepository, RepoError};
use sqlx::Row;

use crate::store::{PgStore, map_err};

fn row_to_state(r: &sqlx::postgres::PgRow) -> ReadState {
    ReadState {
        channel_id: ChannelId(Snowflake::from_raw(r.get::<i64, _>("channel_id") as u64)),
        last_read_message_id: r
            .get::<Option<i64>, _>("last_read_message_id")
            .map(|m| MessageId(Snowflake::from_raw(m as u64))),
        mention_count: r.get::<i32, _>("mention_count"),
    }
}

impl ReadStateRepository for PgStore {
    async fn ack(
        &self,
        user: UserId,
        channel: ChannelId,
        message: MessageId,
    ) -> Result<ReadState, RepoError> {
        // last_read = message, mention_count = 그 이후 살아있는 멘션 수 (한 문장 upsert).
        let row = sqlx::query(
            "INSERT INTO read_states (user_id, channel_id, last_read_message_id, mention_count)
             VALUES (
                 $1, $2, $3,
                 (SELECT count(*) FROM message_mentions mm
                  JOIN messages m ON m.id = mm.message_id
                  WHERE mm.user_id = $1 AND m.channel_id = $2 AND m.id > $3 AND m.deleted_at IS NULL)
             )
             ON CONFLICT (user_id, channel_id) DO UPDATE
                 SET last_read_message_id = EXCLUDED.last_read_message_id,
                     mention_count = EXCLUDED.mention_count
             RETURNING channel_id, last_read_message_id, mention_count",
        )
        .bind(user.0.raw() as i64)
        .bind(channel.0.raw() as i64)
        .bind(message.0.raw() as i64)
        .fetch_one(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(row_to_state(&row))
    }

    async fn bump_mentions(&self, channel: ChannelId, users: &[UserId]) -> Result<(), RepoError> {
        if users.is_empty() {
            return Ok(());
        }
        let ids: Vec<i64> = users.iter().map(|u| u.0.raw() as i64).collect();
        // 존재 유저만(FK 안전) +1 upsert.
        sqlx::query(
            "INSERT INTO read_states (user_id, channel_id, mention_count)
             SELECT u, $2, 1 FROM UNNEST($1::bigint[]) AS u
             WHERE EXISTS (SELECT 1 FROM users WHERE id = u)
             ON CONFLICT (user_id, channel_id)
                 DO UPDATE SET mention_count = read_states.mention_count + 1",
        )
        .bind(&ids)
        .bind(channel.0.raw() as i64)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(())
    }

    async fn list_read_states(&self, user: UserId) -> Result<Vec<ReadState>, RepoError> {
        let rows = sqlx::query(
            "SELECT channel_id, last_read_message_id, mention_count FROM read_states WHERE user_id = $1",
        )
        .bind(user.0.raw() as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(rows.iter().map(row_to_state).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PgStore, connect, run_migrations};
    use domain::message::NewMessage;
    use domain::repo::MessageRepository;

    async fn mk_user(pool: &sqlx::PgPool, id: i64, name: &str) {
        sqlx::query(
            "INSERT INTO users (id, username, email, password_hash)
             VALUES ($1, $2, $3, 'h') ON CONFLICT (id) DO NOTHING",
        )
        .bind(id)
        .bind(name)
        .bind(format!("{name}@e.com"))
        .execute(pool)
        .await
        .unwrap();
    }

    /// 실제 Postgres 필요 — skip if no DATABASE_URL. bump 멘션 → ack 재계산.
    #[tokio::test]
    async fn read_state_bump_and_ack() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL 미설정 — read_state 통합 테스트 skip");
            return;
        };
        let pool = connect(&url).await.expect("connect");
        run_migrations(&pool).await.expect("migrate");
        let store = PgStore::new(pool.clone());

        let (author, me) = (9_600_000_001i64, 9_600_000_002i64);
        let realm = 9_600_000_010i64;
        let chan = ChannelId(Snowflake::from_raw(9_600_000_011));
        mk_user(&pool, author, "rs_author").await;
        mk_user(&pool, me, "rs_me").await;
        let me_uid = UserId(Snowflake::from_raw(me as u64));
        sqlx::query("DELETE FROM realms WHERE id = $1").bind(realm).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO realms (id, kind) VALUES ($1, 'dm')").bind(realm).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO channels (id, realm_id, kind) VALUES ($1, $2, 'dm')")
            .bind(chan.0.raw() as i64).bind(realm).execute(&pool).await.unwrap();
        // messages/message_mentions는 realm FK CASCADE 대상이 아니라 이전 실행 잔여 정리 필요.
        sqlx::query("DELETE FROM message_mentions WHERE message_id = ANY($1)")
            .bind(vec![9_600_001_000i64, 9_600_001_001]).execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM messages WHERE channel_id = $1").bind(chan.0.raw() as i64).execute(&pool).await.unwrap();

        // author가 me를 멘션하는 메시지 2개 적재 + 멘션 행.
        let mut mids = vec![];
        for i in 0..2 {
            let mid = MessageId(Snowflake::from_raw(9_600_001_000 + i));
            store
                .create_message(&NewMessage {
                    id: mid,
                    channel_id: chan,
                    realm_id: domain::id::RealmId(Snowflake::from_raw(realm as u64)),
                    author_id: UserId(Snowflake::from_raw(author as u64)),
                    content: format!("<@{me}> hi {i}"),
                    nonce: None,
                    reference_message_id: None,
                })
                .await
                .unwrap();
            store.add_mentions(mid, &[me_uid]).await.unwrap();
            mids.push(mid);
        }

        // 멘션 발생 → mention_count +2.
        store.bump_mentions(chan, &[me_uid]).await.unwrap();
        store.bump_mentions(chan, &[me_uid]).await.unwrap();
        let states = store.list_read_states(me_uid).await.unwrap();
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].mention_count, 2);
        assert_eq!(states[0].last_read_message_id, None);

        // 첫 메시지까지 ack → 그 이후 멘션 1개 남음.
        let s = store.ack(me_uid, chan, mids[0]).await.unwrap();
        assert_eq!(s.last_read_message_id, Some(mids[0]));
        assert_eq!(s.mention_count, 1);

        // 마지막까지 ack → 0.
        let s = store.ack(me_uid, chan, mids[1]).await.unwrap();
        assert_eq!(s.mention_count, 0);

        sqlx::query("DELETE FROM message_mentions WHERE message_id = ANY($1)")
            .bind(vec![9_600_001_000i64, 9_600_001_001]).execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM messages WHERE channel_id = $1").bind(chan.0.raw() as i64).execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM realms WHERE id = $1").bind(realm).execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM users WHERE id = ANY($1)").bind(vec![author, me]).execute(&pool).await.unwrap();
    }
}
