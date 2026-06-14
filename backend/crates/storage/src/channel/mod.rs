//! `ChannelRepository` 구현 for `PgStore` (개념: channel). adapter (D22).
//! PG `channel_kind` enum은 `$n::channel_kind` 캐스트 / 조회 시 `kind::text`로 주고받음.

use domain::channel::{Channel, ChannelKind, NewChannel};
use domain::id::{ChannelId, RealmId, Snowflake};
use domain::repo::{ChannelRepository, RepoError};
use sqlx::Row;

use crate::store::{PgStore, map_err};

fn row_to_channel(r: &sqlx::postgres::PgRow) -> Channel {
    let id: i64 = r.get("id");
    let realm_id: i64 = r.get("realm_id");
    let kind: String = r.get("kind");
    let name: Option<String> = r.get("name");
    let position: i32 = r.get("position");
    Channel {
        id: ChannelId(Snowflake::from_raw(id as u64)),
        realm_id: RealmId(Snowflake::from_raw(realm_id as u64)),
        kind: ChannelKind::parse(&kind).unwrap_or(ChannelKind::Text),
        name,
        position,
    }
}

impl ChannelRepository for PgStore {
    async fn create_channel(&self, c: &NewChannel) -> Result<(), RepoError> {
        sqlx::query(
            "INSERT INTO channels (id, realm_id, kind, name) VALUES ($1, $2, $3::channel_kind, $4)",
        )
        .bind(c.id.0.raw() as i64)
        .bind(c.realm_id.0.raw() as i64)
        .bind(c.kind.as_str())
        .bind(&c.name)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(())
    }

    async fn get(&self, id: ChannelId) -> Result<Option<Channel>, RepoError> {
        let row = sqlx::query(
            "SELECT id, realm_id, kind::text AS kind, name, position FROM channels \
             WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(id.0.raw() as i64)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(row.as_ref().map(row_to_channel))
    }

    async fn list_by_realm(&self, realm_id: RealmId) -> Result<Vec<Channel>, RepoError> {
        let rows = sqlx::query(
            "SELECT id, realm_id, kind::text AS kind, name, position FROM channels \
             WHERE realm_id = $1 AND deleted_at IS NULL ORDER BY position, id",
        )
        .bind(realm_id.0.raw() as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(rows.iter().map(row_to_channel).collect())
    }
}
