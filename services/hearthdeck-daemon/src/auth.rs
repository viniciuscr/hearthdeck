use chrono::{DateTime, Duration, Utc};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

#[derive(Clone)]
pub struct AuthRepository {
    pool: SqlitePool,
}

impl AuthRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create_pairing_code(
        &self,
        code_hash: String,
    ) -> Result<DateTime<Utc>, sqlx::Error> {
        let expires_at = Utc::now() + Duration::minutes(5);
        sqlx::query("INSERT INTO pairing_sessions (code_hash, expires_at) VALUES (?, ?)")
            .bind(code_hash)
            .bind(expires_at.to_rfc3339())
            .execute(&self.pool)
            .await?;
        Ok(expires_at)
    }

    pub async fn consume_pairing_code(
        &self,
        code_hash: &str,
        client_name: String,
        token_hash: String,
    ) -> Result<Option<PairedClient>, sqlx::Error> {
        let mut transaction = self.pool.begin().await?;
        let row =
            sqlx::query("SELECT expires_at, consumed_at FROM pairing_sessions WHERE code_hash = ?")
                .bind(code_hash)
                .fetch_optional(&mut *transaction)
                .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let expires_at = row.get::<String, _>("expires_at").parse::<DateTime<Utc>>();
        let Ok(expires_at) = expires_at else {
            return Ok(None);
        };
        let consumed_at: Option<String> = row.get("consumed_at");
        if consumed_at.is_some() || expires_at < Utc::now() {
            return Ok(None);
        }

        let client_id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        sqlx::query("UPDATE pairing_sessions SET consumed_at = ? WHERE code_hash = ?")
            .bind(&now)
            .bind(code_hash)
            .execute(&mut *transaction)
            .await?;
        sqlx::query(
            "INSERT INTO paired_clients (id, name, token_hash, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(&client_id)
        .bind(client_name)
        .bind(token_hash)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(Some(PairedClient { client_id }))
    }

    pub async fn authenticate(&self, token_hash: String) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("UPDATE paired_clients SET last_seen_at = ? WHERE token_hash = ?")
            .bind(Utc::now().to_rfc3339())
            .bind(token_hash)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() == 1)
    }
}

pub struct PairedClient {
    pub client_id: String,
}
