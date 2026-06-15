//! `ReactionRepository` 구현 for `PgStore` (개념: reaction). adapter (D39, V7 `reactions`).
//!
//! 유니코드 emoji 단일 컬럼 PK `(message_id, user_id, emoji)`. 추가는 멱등(ON CONFLICT DO NOTHING).

use domain::id::{MessageId, UserId};
use domain::repo::{ReactionRepository, RepoError};

use crate::store::{PgStore, map_err};

impl ReactionRepository for PgStore {
    async fn add_reaction(
        &self,
        message_id: MessageId,
        user: UserId,
        emoji: &str,
    ) -> Result<bool, RepoError> {
        let res = sqlx::query(
            "INSERT INTO reactions (message_id, user_id, emoji) VALUES ($1, $2, $3) \
             ON CONFLICT DO NOTHING",
        )
        .bind(message_id.0.raw() as i64)
        .bind(user.0.raw() as i64)
        .bind(emoji)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(res.rows_affected() > 0)
    }

    async fn remove_reaction(
        &self,
        message_id: MessageId,
        user: UserId,
        emoji: &str,
    ) -> Result<bool, RepoError> {
        let res = sqlx::query(
            "DELETE FROM reactions WHERE message_id = $1 AND user_id = $2 AND emoji = $3",
        )
        .bind(message_id.0.raw() as i64)
        .bind(user.0.raw() as i64)
        .bind(emoji)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(res.rows_affected() > 0)
    }
}
