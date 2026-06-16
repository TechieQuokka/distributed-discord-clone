//! `WebAuthnRepository` 구현 for `PgStore` (개념: webauthn). adapter (D22, D19).
//! `passkey`는 jsonb지만 text로 주고받아 storage가 serde 무의존(auth가 직렬화한 문자열 통과).

use domain::id::UserId;
use domain::repo::{RepoError, WebAuthnRepository};
use domain::webauthn::{NewWebAuthnCredential, WebAuthnCredential};
use sqlx::Row;

use crate::store::{PgStore, map_err};

impl WebAuthnRepository for PgStore {
    async fn add_credential(&self, c: &NewWebAuthnCredential) -> Result<(), RepoError> {
        sqlx::query(
            "INSERT INTO webauthn_credentials (id, user_id, credential_id, passkey) \
             VALUES ($1, $2, $3, $4::jsonb)",
        )
        .bind(c.id as i64)
        .bind(c.user_id.0.raw() as i64)
        .bind(&c.credential_id)
        .bind(&c.passkey_json)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(())
    }

    async fn list_credentials(&self, user: UserId) -> Result<Vec<WebAuthnCredential>, RepoError> {
        let rows = sqlx::query(
            "SELECT credential_id, passkey::text AS passkey \
             FROM webauthn_credentials WHERE user_id = $1 ORDER BY id",
        )
        .bind(user.0.raw() as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(rows
            .iter()
            .map(|r| WebAuthnCredential {
                credential_id: r.get("credential_id"),
                passkey_json: r.get("passkey"),
            })
            .collect())
    }

    async fn update_credential(&self, credential_id: &[u8], passkey_json: &str) -> Result<(), RepoError> {
        sqlx::query("UPDATE webauthn_credentials SET passkey = $2::jsonb WHERE credential_id = $1")
            .bind(credential_id)
            .bind(passkey_json)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PgStore, connect, run_migrations};
    use domain::id::Snowflake;
    use domain::repo::UserRepository;
    use domain::user::NewUser;

    /// 실제 Postgres 필요 — skip if no DATABASE_URL. 자격증명 저장/목록/counter 갱신.
    #[tokio::test]
    async fn webauthn_credential_roundtrip() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL 미설정 — webauthn 테스트 skip");
            return;
        };
        let pool = connect(&url).await.expect("connect");
        run_migrations(&pool).await.expect("migrate");
        let s = PgStore::new(pool.clone());

        let uid = UserId(Snowflake::from_raw(880_001));
        sqlx::query("DELETE FROM users WHERE id = $1").bind(uid.0.raw() as i64).execute(&pool).await.unwrap();
        s.create_user(&NewUser { id: uid, username: "wa_user".into(), email: "wa@e.com".into(), password_hash: "x".into() }).await.unwrap();

        s.add_credential(&NewWebAuthnCredential {
            id: 880_010,
            user_id: uid,
            credential_id: vec![1, 2, 3, 4],
            passkey_json: r#"{"counter":0}"#.into(),
        })
        .await
        .unwrap();

        let creds = s.list_credentials(uid).await.unwrap();
        assert_eq!(creds.len(), 1);
        assert_eq!(creds[0].credential_id, vec![1, 2, 3, 4]);
        assert!(creds[0].passkey_json.contains("counter"));

        // counter 갱신.
        s.update_credential(&[1, 2, 3, 4], r#"{"counter":5}"#).await.unwrap();
        let creds = s.list_credentials(uid).await.unwrap();
        assert!(creds[0].passkey_json.contains("5"));

        sqlx::query("DELETE FROM users WHERE id = $1").bind(uid.0.raw() as i64).execute(&pool).await.unwrap();
    }
}
