use super::{AccountError, Accounts};
use crate::tokens;
use uuid::Uuid;

impl Accounts {
    /// Consume a purpose-bound reset and revoke all account credentials atomically.
    /// No SMTP/provider request belongs in this transaction: provider cleanup is
    /// durable, while local revocation is authoritative immediately after commit.
    pub async fn reset_password(&self, token: &str, password: &str) -> Result<(), AccountError> {
        if !self.email_enabled {
            return Err(AccountError::EmailDisabled);
        }
        let digest = tokens::digest(token).ok_or(AccountError::InvalidToken)?;
        // Bound hashing outside the transaction so locks do not wait on CPU work.
        let hash = self.passwords.hash(password).await?;
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| AccountError::Unavailable)?;
        let user_id: Option<Uuid> = sqlx::query_scalar(
            "SELECT user_id FROM account_tokens WHERE token_digest = $1 \
             AND purpose = 'password_reset' AND consumed_at IS NULL \
             AND expires_at > clock_timestamp()",
        )
        .bind(&digest)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| AccountError::Unavailable)?;
        let user_id = user_id.ok_or(AccountError::InvalidToken)?;
        // Same lock ordering as login, reauthentication, verification, and delivery.
        sqlx::query("SELECT id FROM users WHERE id = $1 FOR UPDATE")
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|_| AccountError::Unavailable)?;
        let consumed = sqlx::query(
            "UPDATE account_tokens SET consumed_at = clock_timestamp() \
             WHERE token_digest = $1 AND user_id = $2 AND purpose = 'password_reset' \
             AND consumed_at IS NULL AND expires_at > clock_timestamp()",
        )
        .bind(&digest)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(|_| AccountError::Unavailable)?;
        if consumed.rows_affected() != 1 {
            return Err(AccountError::InvalidToken);
        }
        sqlx::query("UPDATE users SET password_hash = $1 WHERE id = $2")
            .bind(hash)
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(|_| AccountError::Unavailable)?;
        sqlx::query(
            "UPDATE account_tokens SET consumed_at = clock_timestamp() \
             WHERE user_id = $1 AND purpose = 'password_reset' AND consumed_at IS NULL",
        )
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(|_| AccountError::Unavailable)?;
        sqlx::query(
            "UPDATE sessions SET revoked_at = coalesce(revoked_at, clock_timestamp()) WHERE user_id = $1",
        ).bind(user_id).execute(&mut *tx).await.map_err(|_| AccountError::Unavailable)?;
        // One set-based statement avoids unbounded in-memory key lists and
        // enqueues precisely the keys newly revoked by this transaction.
        sqlx::query(
            "WITH revoked AS ( \
                 UPDATE api_keys SET revoked_at = clock_timestamp() \
                 WHERE user_id = $1 AND revoked_at IS NULL RETURNING key_id \
             ) INSERT INTO delivery_jobs (id, kind, key_id) \
                 SELECT gen_random_uuid(), 'unkey_revoke', key_id FROM revoked",
        )
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(|_| AccountError::Unavailable)?;
        tx.commit().await.map_err(|_| AccountError::Unavailable)
    }
}
