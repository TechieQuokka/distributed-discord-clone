//! `ChannelOverwriteRepository` 구현 for `PgStore` (개념: overwrite). adapter (D17/D22).
//!
//! target_type은 Postgres `overwrite_kind` enum — 바인딩 시 텍스트 캐스트(`::overwrite_kind`).

use domain::id::ChannelId;
use domain::permissions::{ChannelOverwrite, OverwriteKind, Permissions};
use domain::repo::{ChannelOverwriteRepository, RepoError};
use sqlx::Row;

use crate::store::{PgStore, map_err};

impl ChannelOverwriteRepository for PgStore {
    async fn set_overwrite(&self, ow: &ChannelOverwrite) -> Result<(), RepoError> {
        sqlx::query(
            "INSERT INTO channel_overwrites (channel_id, target_id, target_type, allow, deny)
             VALUES ($1, $2, $3::overwrite_kind, $4, $5)
             ON CONFLICT (channel_id, target_id)
             DO UPDATE SET target_type = EXCLUDED.target_type, allow = EXCLUDED.allow, deny = EXCLUDED.deny",
        )
        .bind(ow.channel_id.0.raw() as i64)
        .bind(ow.target_id as i64)
        .bind(ow.kind.as_str())
        .bind(ow.allow.bits() as i64)
        .bind(ow.deny.bits() as i64)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(())
    }

    async fn list_overwrites(&self, channel_id: ChannelId) -> Result<Vec<ChannelOverwrite>, RepoError> {
        let rows = sqlx::query(
            "SELECT target_id, target_type::text AS kind, allow, deny
             FROM channel_overwrites WHERE channel_id = $1",
        )
        .bind(channel_id.0.raw() as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(rows
            .iter()
            .map(|r| ChannelOverwrite {
                channel_id,
                target_id: r.get::<i64, _>("target_id") as u64,
                kind: OverwriteKind::parse(r.get::<&str, _>("kind")).unwrap_or(OverwriteKind::Role),
                allow: Permissions::from_bits_truncate(r.get::<i64, _>("allow") as u64),
                deny: Permissions::from_bits_truncate(r.get::<i64, _>("deny") as u64),
            })
            .collect())
    }
}
