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
        let now = Utc::now();
        // Drop codes that can no longer be redeemed before adding another: a
        // consumed one is spent, and an expired one can never match again. Without
        // this the table grows for the life of the install, one row per pairing
        // attempt.
        sqlx::query("DELETE FROM pairing_sessions WHERE consumed_at IS NOT NULL OR expires_at < ?")
            .bind(now.to_rfc3339())
            .execute(&self.pool)
            .await?;
        let expires_at = now + Duration::minutes(5);
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
        // Claim the code atomically: the `consumed_at IS NULL` predicate makes the
        // update itself the test-and-set, so two concurrent redemptions cannot both
        // pass the read above and each mint a client. The read is not the guard; the
        // affected-row count is.
        let consumed = sqlx::query(
            "UPDATE pairing_sessions SET consumed_at = ? WHERE code_hash = ? AND consumed_at IS NULL",
        )
        .bind(&now)
        .bind(code_hash)
        .execute(&mut *transaction)
        .await?;
        if consumed.rows_affected() != 1 {
            // Someone else redeemed it first; dropping the transaction rolls back.
            return Ok(None);
        }
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
        // A read, not a write. This runs on every authenticated request, and the
        // frontend polls several endpoints a second, so a `UPDATE ... SET
        // last_seen_at` here turned every read into a write-lock acquisition and
        // serialized the whole API on SQLite's single writer. `last_seen_at` is
        // written nowhere else and read nowhere at all, so the write bought nothing.
        let row = sqlx::query("SELECT 1 FROM paired_clients WHERE token_hash = ?")
            .bind(token_hash)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.is_some())
    }
}

pub struct PairedClient {
    pub client_id: String,
}

#[cfg(test)]
mod tests {
    use sqlx::Row;

    use super::AuthRepository;
    use crate::database::Database;

    #[tokio::test]
    async fn creating_a_code_prunes_spent_and_expired_ones() {
        let directory = tempfile::tempdir().unwrap();
        let database = Database::connect(&directory.path().join("hearthdeck.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let auth = AuthRepository::new(database.pool().clone());

        // One code redeemed, and one that has already expired.
        auth.create_pairing_code("spent".to_owned()).await.unwrap();
        sqlx::query(
            "UPDATE pairing_sessions SET consumed_at = '2020-01-01T00:00:00Z' WHERE code_hash = 'spent'",
        )
        .execute(database.pool())
        .await
        .unwrap();
        sqlx::query("INSERT INTO pairing_sessions (code_hash, expires_at) VALUES (?, ?)")
            .bind("expired")
            .bind((chrono::Utc::now() - chrono::Duration::minutes(10)).to_rfc3339())
            .execute(database.pool())
            .await
            .unwrap();

        // Minting another prunes both, leaving only the live code.
        auth.create_pairing_code("fresh".to_owned()).await.unwrap();

        let remaining: Vec<String> = sqlx::query("SELECT code_hash FROM pairing_sessions")
            .fetch_all(database.pool())
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.get::<String, _>("code_hash"))
            .collect();
        assert_eq!(remaining, vec!["fresh".to_owned()]);
    }
}
