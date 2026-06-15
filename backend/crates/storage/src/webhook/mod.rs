//! `WebhookRepository` 구현 for `PgStore` (개념: webhook). adapter (D22).

use domain::id::{ChannelId, RealmId, Snowflake, UserId, WebhookId};
use domain::repo::{RepoError, WebhookRepository};
use domain::webhook::{NewWebhook, Webhook};
use sqlx::Row;

use crate::store::{PgStore, map_err};

fn row_to_webhook(r: &sqlx::postgres::PgRow) -> Webhook {
    let id: i64 = r.get("id");
    let channel_id: i64 = r.get("channel_id");
    let realm_id: i64 = r.get("realm_id");
    let creator: Option<i64> = r.get("creator_id");
    Webhook {
        id: WebhookId(Snowflake::from_raw(id as u64)),
        channel_id: ChannelId(Snowflake::from_raw(channel_id as u64)),
        realm_id: RealmId(Snowflake::from_raw(realm_id as u64)),
        name: r.get("name"),
        creator_id: creator.map(|c| UserId(Snowflake::from_raw(c as u64))),
        token_hash: r.get("token_hash"),
    }
}

impl WebhookRepository for PgStore {
    async fn create_webhook(&self, w: &NewWebhook) -> Result<(), RepoError> {
        sqlx::query(
            "INSERT INTO webhooks (id, channel_id, realm_id, name, token_hash, creator_id) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(w.id.0.raw() as i64)
        .bind(w.channel_id.0.raw() as i64)
        .bind(w.realm_id.0.raw() as i64)
        .bind(&w.name)
        .bind(&w.token_hash)
        .bind(w.creator_id.0.raw() as i64)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(())
    }

    async fn list_webhooks(&self, channel_id: ChannelId) -> Result<Vec<Webhook>, RepoError> {
        let rows = sqlx::query(
            "SELECT id, channel_id, realm_id, name, token_hash, creator_id FROM webhooks \
             WHERE channel_id = $1 ORDER BY id",
        )
        .bind(channel_id.0.raw() as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(rows.iter().map(row_to_webhook).collect())
    }

    async fn get_webhook(&self, id: WebhookId) -> Result<Option<Webhook>, RepoError> {
        let row = sqlx::query(
            "SELECT id, channel_id, realm_id, name, token_hash, creator_id FROM webhooks WHERE id = $1",
        )
        .bind(id.0.raw() as i64)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(row.as_ref().map(row_to_webhook))
    }

    async fn delete_webhook(&self, id: WebhookId) -> Result<bool, RepoError> {
        let res = sqlx::query("DELETE FROM webhooks WHERE id = $1")
            .bind(id.0.raw() as i64)
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
    use domain::repo::{ChannelRepository, GuildRepository, UserRepository};
    use domain::user::NewUser;

    /// 실제 Postgres 필요 — skip if no DATABASE_URL. 웹훅 생성/목록/조회/삭제.
    #[tokio::test]
    async fn webhook_lifecycle() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL 미설정 — webhook 테스트 skip");
            return;
        };
        let pool = connect(&url).await.expect("connect");
        run_migrations(&pool).await.expect("migrate");
        let s = PgStore::new(pool.clone());

        let owner = UserId(Snowflake::from_raw(760_001));
        let realm = RealmId(Snowflake::from_raw(760_002));
        let chan = ChannelId(Snowflake::from_raw(760_003));
        let wid = WebhookId(Snowflake::from_raw(760_010));
        sqlx::query("DELETE FROM realms WHERE id = $1").bind(realm.0.raw() as i64).execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM users WHERE id = $1").bind(owner.0.raw() as i64).execute(&pool).await.unwrap();

        s.create_user(&NewUser { id: owner, username: "wh_owner".into(), email: "wh@e.com".into(), password_hash: "x".into() }).await.unwrap();
        s.create_guild(&NewGuild { realm_id: realm, name: "G".into(), owner_id: owner }).await.unwrap();
        s.create_channel(&NewChannel { id: chan, realm_id: realm, kind: ChannelKind::Text, name: "general".into() }).await.unwrap();

        // token_hash 해싱은 rest-api(auth)의 책임 — storage는 바이트만 저장/반환.
        let hash = vec![1u8, 2, 3, 4, 5];
        s.create_webhook(&NewWebhook { id: wid, channel_id: chan, realm_id: realm, name: "ci-bot".into(), creator_id: owner, token_hash: hash.clone() }).await.unwrap();

        let list = s.list_webhooks(chan).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "ci-bot");

        let got = s.get_webhook(wid).await.unwrap().unwrap();
        assert_eq!(got.token_hash, hash, "저장 바이트 그대로 반환");
        assert_eq!(got.channel_id, chan);

        assert!(s.delete_webhook(wid).await.unwrap());
        assert!(s.get_webhook(wid).await.unwrap().is_none());
        assert!(!s.delete_webhook(wid).await.unwrap(), "이미 삭제 → false");

        sqlx::query("DELETE FROM realms WHERE id = $1").bind(realm.0.raw() as i64).execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM users WHERE id = $1").bind(owner.0.raw() as i64).execute(&pool).await.unwrap();
    }
}
