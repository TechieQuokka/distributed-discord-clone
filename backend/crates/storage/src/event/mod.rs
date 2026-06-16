//! `EventLogRepository` 구현 for `PgStore` (개념: event). 이벤트 소싱 adapter (D23/D48).
//!
//! 타입화된 `RealmEventKind`를 (code + nullable bigint 슬롯)으로 매핑/역매핑 — serde 무의존(audit와 정합).
//! per-realm seq는 `coalesce(max(seq),0)+1`로 부여(단일 직렬 소비자 D24라 레이스 없음, nonce D34 동형).

use domain::event::{RealmEventKind, RealmEventRecord};
use domain::id::{ChannelId, MessageId, RealmId, Snowflake, UserId};
use domain::repo::{EventLogRepository, RepoError};
use sqlx::Row;

use crate::store::{PgStore, map_err};

/// enum → (code, message_id?, channel_id?, user_id?) 슬롯.
fn encode(kind: &RealmEventKind) -> (i16, Option<i64>, Option<i64>, Option<i64>) {
    let id = |s: Snowflake| s.raw() as i64;
    match kind {
        RealmEventKind::MessageCreated { message_id, channel_id, author } => {
            (1, Some(id(message_id.0)), Some(id(channel_id.0)), Some(id(author.0)))
        }
        RealmEventKind::MessageDeleted { message_id, channel_id } => {
            (2, Some(id(message_id.0)), Some(id(channel_id.0)), None)
        }
        RealmEventKind::MemberJoined { user } => (3, None, None, Some(id(user.0))),
        RealmEventKind::MemberLeft { user } => (4, None, None, Some(id(user.0))),
    }
}

fn sf(v: i64) -> Snowflake {
    Snowflake::from_raw(v as u64)
}

/// (code + 슬롯) → enum. 슬롯 결손은 손상된 로그 → Backend 에러.
fn decode(
    code: i16,
    message_id: Option<i64>,
    channel_id: Option<i64>,
    user_id: Option<i64>,
) -> Result<RealmEventKind, RepoError> {
    let need = |o: Option<i64>, f: &str| {
        o.ok_or_else(|| RepoError::Backend(format!("realm_events code {code} missing {f}")))
    };
    Ok(match code {
        1 => RealmEventKind::MessageCreated {
            message_id: MessageId(sf(need(message_id, "message_id")?)),
            channel_id: ChannelId(sf(need(channel_id, "channel_id")?)),
            author: UserId(sf(need(user_id, "user_id")?)),
        },
        2 => RealmEventKind::MessageDeleted {
            message_id: MessageId(sf(need(message_id, "message_id")?)),
            channel_id: ChannelId(sf(need(channel_id, "channel_id")?)),
        },
        3 => RealmEventKind::MemberJoined { user: UserId(sf(need(user_id, "user_id")?)) },
        4 => RealmEventKind::MemberLeft { user: UserId(sf(need(user_id, "user_id")?)) },
        other => return Err(RepoError::Backend(format!("unknown realm_events code {other}"))),
    })
}

impl EventLogRepository for PgStore {
    async fn append_event(
        &self,
        realm: RealmId,
        kind: &RealmEventKind,
    ) -> Result<u64, RepoError> {
        let (code, message_id, channel_id, user_id) = encode(kind);
        // per-realm 단조 seq: 같은 realm append는 단일 직렬 소비자(D24)뿐 → 레이스 없음.
        let seq: i64 = sqlx::query_scalar(
            "INSERT INTO realm_events (realm_id, seq, code, message_id, channel_id, user_id) \
             VALUES ($1, (SELECT coalesce(max(seq), 0) + 1 FROM realm_events WHERE realm_id = $1), \
                     $2, $3, $4, $5) \
             RETURNING seq",
        )
        .bind(realm.0.raw() as i64)
        .bind(code)
        .bind(message_id)
        .bind(channel_id)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(seq as u64)
    }

    async fn replay_events(
        &self,
        realm: RealmId,
        after_seq: u64,
    ) -> Result<Vec<RealmEventRecord>, RepoError> {
        let rows = sqlx::query(
            "SELECT seq, code, message_id, channel_id, user_id FROM realm_events \
             WHERE realm_id = $1 AND seq > $2 ORDER BY seq ASC",
        )
        .bind(realm.0.raw() as i64)
        .bind(after_seq as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;

        rows.iter()
            .map(|r| {
                let seq: i64 = r.get("seq");
                let code: i16 = r.get("code");
                let kind = decode(code, r.get("message_id"), r.get("channel_id"), r.get("user_id"))?;
                Ok(RealmEventRecord { realm_id: realm, seq: seq as u64, kind })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{connect, run_migrations};
    use domain::event::RealmProjection;

    fn rid(n: u64) -> RealmId {
        RealmId(sf(n as i64))
    }
    fn mid(n: u64) -> MessageId {
        MessageId(sf(n as i64))
    }
    fn cid(n: u64) -> ChannelId {
        ChannelId(sf(n as i64))
    }
    fn uid(n: u64) -> UserId {
        UserId(sf(n as i64))
    }

    /// 실제 Postgres 필요 — `DATABASE_URL` 미설정 시 skip. append→replay→projection 라운드트립:
    /// 저장한 이벤트를 재생해 프로젝션이 정확히 재구성되는지(이벤트 소싱 핵심) 검증.
    #[tokio::test]
    async fn append_replay_reconstructs_projection() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL 미설정 — event 통합 테스트 skip");
            return;
        };
        let pool = connect(&url).await.expect("connect");
        run_migrations(&pool).await.expect("migrate");
        let store = PgStore::new(pool);

        // 다른 테스트와 충돌 안 나게 고유 realm id 사용 후 정리.
        let realm = rid(0x5EED_0000_0001);
        sqlx::query("DELETE FROM realm_events WHERE realm_id = $1")
            .bind(realm.0.raw() as i64)
            .execute(&store.pool)
            .await
            .unwrap();

        let events = [
            RealmEventKind::MemberJoined { user: uid(10) },
            RealmEventKind::MemberJoined { user: uid(20) },
            RealmEventKind::MessageCreated { message_id: mid(1000), channel_id: cid(5), author: uid(10) },
            RealmEventKind::MessageCreated { message_id: mid(2000), channel_id: cid(5), author: uid(20) },
            RealmEventKind::MessageDeleted { message_id: mid(1000), channel_id: cid(5) },
            RealmEventKind::MemberLeft { user: uid(10) },
        ];
        // append → per-realm seq가 1..6 단조여야.
        for (i, e) in events.iter().enumerate() {
            let seq = store.append_event(realm, e).await.expect("append");
            assert_eq!(seq, (i + 1) as u64, "seq 단조 부여");
        }

        // replay → projection 재구성.
        let log = store.replay_events(realm, 0).await.expect("replay");
        assert_eq!(log.len(), 6);
        let p = RealmProjection::replay(&log);
        assert_eq!(p.members, std::collections::BTreeSet::from([20]));
        assert_eq!(p.message_count, 1);
        assert_eq!(p.last_message_id, Some(2000));
        assert_eq!(p.last_seq, 6);

        // 증분 재생: seq>4 이후만.
        let tail = store.replay_events(realm, 4).await.expect("replay tail");
        assert_eq!(tail.len(), 2, "seq 5,6만");
        assert_eq!(tail[0].seq, 5);

        sqlx::query("DELETE FROM realm_events WHERE realm_id = $1")
            .bind(realm.0.raw() as i64)
            .execute(&store.pool)
            .await
            .unwrap();
    }
}
