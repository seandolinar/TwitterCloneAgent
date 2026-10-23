use super::{
    AccountError, AccountIdentity, Accounts, LoginRow, credential_email, validate_password,
};
use crate::{
    config::Config,
    tokens::{self, BearerToken},
};
use cookie::{Cookie, SameSite, time::Duration};
use sqlx::PgPool;

// No Debug/Serialize: the raw bearer value is only for the cookie boundary.
pub struct SessionGrant {
    pub identity: AccountIdentity,
    pub token: BearerToken,
}

pub struct ActiveSession {
    pub identity: AccountIdentity,
    pub recent_password_auth: bool,
    digest: Vec<u8>,
}

impl ActiveSession {
    pub fn binding(&self) -> &[u8] {
        &self.digest
    }
}

impl SessionGrant {
    pub fn cookie(&self, config: &Config) -> String {
        Cookie::build((session_cookie_name(config), self.token.expose_secret()))
            .http_only(true)
            .secure(config.cookie_secure)
            .same_site(SameSite::Lax)
            .path("/")
            .max_age(Duration::days(7))
            .build()
            .to_string()
    }
}

pub fn session_cookie_name(config: &Config) -> &'static str {
    if config.cookie_secure {
        "__Host-session"
    } else {
        "twitterclone_dev_session"
    }
}

pub fn clear_session_cookie(config: &Config) -> String {
    let mut cookie = Cookie::build((session_cookie_name(config), ""))
        .http_only(true)
        .secure(config.cookie_secure)
        .same_site(SameSite::Lax)
        .path("/")
        .build();
    cookie.make_removal();
    cookie.to_string()
}

#[derive(sqlx::FromRow)]
struct SessionRow {
    id: uuid::Uuid,
    handle: String,
    verified: bool,
    recent: bool,
}

impl Accounts {
    pub async fn login(
        &self,
        email: &str,
        password: &str,
        previous: Option<&str>,
    ) -> Result<SessionGrant, AccountError> {
        let email = credential_email(email, password)?;
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| AccountError::Unavailable)?;
        // Serialize with password reset so an old credential check cannot issue
        // a new session after reset has changed the password/revoked sessions.
        let row: Option<LoginRow> = sqlx::query_as(
            "SELECT id, handle, verified_at IS NOT NULL AS verified, password_hash \
             FROM users WHERE email_normalized = $1 FOR UPDATE",
        )
        .bind(email)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| AccountError::Unavailable)?;
        let identity = self.verify_login_row(row, password).await?;
        if let Some(digest) = previous.and_then(tokens::digest) {
            sqlx::query(
                "UPDATE sessions SET revoked_at = coalesce(revoked_at, clock_timestamp()) \
                 WHERE token_digest = $1",
            )
            .bind(digest)
            .execute(&mut *tx)
            .await
            .map_err(|_| AccountError::Unavailable)?;
        }
        let token = BearerToken::generate()?;
        sqlx::query(
            "INSERT INTO sessions (token_digest, user_id, expires_at) \
             VALUES ($1, $2, now() + interval '7 days')",
        )
        .bind(token.digest())
        .bind(identity.id)
        .execute(&mut *tx)
        .await
        .map_err(|_| AccountError::Unavailable)?;
        tx.commit().await.map_err(|_| AccountError::Unavailable)?;
        Ok(SessionGrant { identity, token })
    }

    pub async fn resolve_session(
        &self,
        token: &str,
    ) -> Result<Option<ActiveSession>, AccountError> {
        let Some(digest) = tokens::digest(token) else {
            return Ok(None);
        };
        let row: Option<SessionRow> = sqlx::query_as(
            "UPDATE sessions AS session SET last_used_at = clock_timestamp() \
             FROM users AS account \
             WHERE session.token_digest = $1 AND session.user_id = account.id \
                 AND session.revoked_at IS NULL AND session.expires_at > clock_timestamp() \
                 AND session.last_used_at > clock_timestamp() - interval '24 hours' \
             RETURNING account.id, account.handle, account.verified_at IS NOT NULL AS verified, \
                 session.password_authenticated_at > clock_timestamp() - interval '5 minutes' AS recent",
        ).bind(&digest).fetch_optional(&self.pool).await.map_err(|_| AccountError::Unavailable)?;
        Ok(row.map(|row| ActiveSession {
            identity: AccountIdentity {
                id: row.id,
                handle: row.handle,
                verified: row.verified,
            },
            recent_password_auth: row.recent,
            digest,
        }))
    }

    pub async fn logout(&self, token: &str) -> Result<(), AccountError> {
        if let Some(digest) = tokens::digest(token) {
            sqlx::query(
                "UPDATE sessions SET revoked_at = coalesce(revoked_at, clock_timestamp()) \
                 WHERE token_digest = $1",
            )
            .bind(digest)
            .execute(&self.pool)
            .await
            .map_err(|_| AccountError::Unavailable)?;
        }
        Ok(())
    }

    pub async fn reauthenticate(
        &self,
        session: &ActiveSession,
        password: &str,
    ) -> Result<(), AccountError> {
        validate_password(password).map_err(|_| AccountError::InvalidCredentials)?;
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| AccountError::Unavailable)?;
        let row: Option<LoginRow> = sqlx::query_as(
            "SELECT id, handle, verified_at IS NOT NULL AS verified, password_hash \
             FROM users WHERE id = $1 FOR UPDATE",
        )
        .bind(session.identity.id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| AccountError::Unavailable)?;
        self.verify_login_row(row, password).await?;
        let updated = sqlx::query(
            "UPDATE sessions SET password_authenticated_at = clock_timestamp() \
             WHERE token_digest = $1 AND user_id = $2 AND revoked_at IS NULL \
                 AND expires_at > clock_timestamp() \
                 AND last_used_at > clock_timestamp() - interval '24 hours'",
        )
        .bind(&session.digest)
        .bind(session.identity.id)
        .execute(&mut *tx)
        .await
        .map_err(|_| AccountError::Unavailable)?;
        if updated.rows_affected() != 1 {
            return Err(AccountError::InvalidCredentials);
        }
        tx.commit().await.map_err(|_| AccountError::Unavailable)
    }
}

pub(crate) async fn cleanup_expired_security_records(
    pool: &PgPool,
    batch: u32,
) -> Result<u64, AccountError> {
    if batch == 0 || batch > 1000 {
        return Err(AccountError::Unavailable);
    }
    let operation = async {
        let sessions = sqlx::query(
            "WITH expired AS (SELECT token_digest FROM sessions \
                 WHERE expires_at <= clock_timestamp() OR revoked_at IS NOT NULL \
                     OR last_used_at <= clock_timestamp() - interval '24 hours' \
                 LIMIT $1 FOR UPDATE SKIP LOCKED) \
             DELETE FROM sessions USING expired WHERE sessions.token_digest = expired.token_digest",
        )
        .bind(i64::from(batch))
        .execute(pool)
        .await?;
        let tokens = sqlx::query(
            "WITH expired AS (SELECT token_digest FROM account_tokens \
                 WHERE expires_at <= clock_timestamp() OR consumed_at IS NOT NULL \
                 LIMIT $1 FOR UPDATE SKIP LOCKED) \
             DELETE FROM account_tokens USING expired \
                 WHERE account_tokens.token_digest = expired.token_digest",
        )
        .bind(i64::from(batch))
        .execute(pool)
        .await?;
        Ok::<_, sqlx::Error>(sessions.rows_affected() + tokens.rows_affected())
    };
    tokio::time::timeout(std::time::Duration::from_secs(2), operation)
        .await
        .map_err(|_| AccountError::Unavailable)?
        .map_err(|_| AccountError::Unavailable)
}
