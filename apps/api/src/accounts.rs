use crate::{
    http::ApiError,
    jobs::{self, Intent},
};
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use email_address::EmailAddress;
use sqlx::PgPool;
use std::{error::Error, fmt, str::FromStr};
use uuid::Uuid;

mod email;
mod passwords;
mod recovery;
mod sessions;
pub use email::EmailDelivery;
pub use passwords::{Passwords, validate_password};
pub(crate) use sessions::cleanup_expired_security_records;
pub use sessions::{ActiveSession, SessionGrant, clear_session_cookie, session_cookie_name};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountError {
    InvalidEmail,
    InvalidHandle,
    InvalidPassword,
    HandleTaken,
    InvalidCredentials,
    InvalidToken,
    Unavailable,
}

impl fmt::Display for AccountError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidEmail => "Enter a valid email address.",
            Self::InvalidHandle => "Handles require 3–30 ASCII letters, digits, or underscores.",
            Self::InvalidPassword => "Passwords require 15–128 Unicode code points.",
            Self::HandleTaken => "That handle is already taken.",
            Self::InvalidCredentials => "Email or password is incorrect.",
            Self::InvalidToken => "This link is invalid or has expired.",
            Self::Unavailable => "The service is temporarily unavailable.",
        })
    }
}

impl Error for AccountError {}

impl IntoResponse for AccountError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            Self::InvalidEmail => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_email",
                "Enter a valid email address.",
            ),
            Self::InvalidHandle => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_handle",
                "Handles require 3–30 ASCII letters, digits, or underscores.",
            ),
            Self::InvalidPassword => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_password",
                "Passwords require 15–128 Unicode code points.",
            ),
            Self::HandleTaken => (
                StatusCode::CONFLICT,
                "handle_taken",
                "That handle is already taken.",
            ),
            Self::InvalidCredentials => (
                StatusCode::UNAUTHORIZED,
                "invalid_credentials",
                "Email or password is incorrect.",
            ),
            Self::InvalidToken => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_token",
                "This link is invalid or has expired.",
            ),
            Self::Unavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "service_unavailable",
                "The service is temporarily unavailable.",
            ),
        };
        ApiError {
            status,
            code,
            message,
        }
        .into_response()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountIdentity {
    pub id: Uuid,
    pub handle: String,
    pub verified: bool,
}

// Keep the hash-bearing row out of diagnostics and public DTOs.
#[derive(sqlx::FromRow)]
struct LoginRow {
    id: Uuid,
    handle: String,
    verified: bool,
    password_hash: String,
}

#[derive(Clone)]
pub struct Accounts {
    pub(crate) pool: PgPool,
    passwords: Passwords,
}

impl Accounts {
    pub async fn new(pool: PgPool) -> Result<Self, AccountError> {
        Ok(Self {
            pool,
            passwords: Passwords::new().await?,
        })
    }

    /// Return the same acknowledgement for a new account and an existing email.
    /// Registration does not authenticate the caller or return a private user ID.
    pub async fn register(
        &self,
        email: &str,
        handle: &str,
        password: &str,
    ) -> Result<(), AccountError> {
        let email = normalize_email(email)?;
        validate_handle(handle)?;
        // Hash before opening a transaction, including duplicate-email attempts.
        let hash = self.passwords.hash(password).await?;
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| AccountError::Unavailable)?;
        let inserted: Option<Uuid> = match sqlx::query_scalar(
            "INSERT INTO users (id, email_normalized, handle, password_hash, display_name) \
             VALUES ($1, $2, $3, $4, $3) \
             ON CONFLICT (email_normalized) DO NOTHING RETURNING id",
        )
        .bind(Uuid::new_v4())
        .bind(email)
        .bind(handle)
        .bind(hash)
        .fetch_optional(&mut *tx)
        .await
        {
            Ok(id) => id,
            Err(error) => {
                if error
                    .as_database_error()
                    .is_some_and(|error| error.constraint() == Some("users_handle_normalized_key"))
                {
                    return Err(AccountError::HandleTaken);
                }
                return Err(AccountError::Unavailable);
            }
        };
        if let Some(user_id) = inserted {
            jobs::enqueue(&mut tx, &Intent::VerificationEmail { user_id })
                .await
                .map_err(|_| AccountError::Unavailable)?;
        }
        tx.commit().await.map_err(|_| AccountError::Unavailable)
    }

    /// Credential checking only; the session/CSRF adapters are separate tasks.
    pub async fn authenticate(
        &self,
        email: &str,
        password: &str,
    ) -> Result<AccountIdentity, AccountError> {
        let email = credential_email(email, password)?;
        let row: Option<LoginRow> = sqlx::query_as(
            "SELECT id, handle, verified_at IS NOT NULL AS verified, password_hash \
             FROM users WHERE email_normalized = $1",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| AccountError::Unavailable)?;
        self.verify_login_row(row, password).await
    }

    async fn verify_login_row(
        &self,
        row: Option<LoginRow>,
        password: &str,
    ) -> Result<AccountIdentity, AccountError> {
        let valid = self
            .passwords
            .verify(password, row.as_ref().map(|row| row.password_hash.as_str()))
            .await?;
        match row {
            Some(row) if valid => Ok(AccountIdentity {
                id: row.id,
                handle: row.handle,
                verified: row.verified,
            }),
            _ => Err(AccountError::InvalidCredentials),
        }
    }
}

fn credential_email(email: &str, password: &str) -> Result<String, AccountError> {
    let normalized = normalize_email(email).map_err(|_| AccountError::InvalidCredentials)?;
    validate_password(password).map_err(|_| AccountError::InvalidCredentials)?;
    Ok(normalized)
}

pub fn normalize_email(input: &str) -> Result<String, AccountError> {
    let normalized = input.trim().to_lowercase();
    if normalized.len() > 254 || normalized.chars().any(char::is_control) {
        return Err(AccountError::InvalidEmail);
    }
    let parsed = EmailAddress::from_str(&normalized).map_err(|_| AccountError::InvalidEmail)?;
    if !parsed.display_part().is_empty() || parsed.email() != normalized {
        return Err(AccountError::InvalidEmail);
    }
    Ok(normalized)
}

pub fn validate_handle(handle: &str) -> Result<(), AccountError> {
    if !(3..=30).contains(&handle.len())
        || !handle
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
    {
        return Err(AccountError::InvalidHandle);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_email_without_provider_specific_alias_rewriting() {
        assert_eq!(
            normalize_email(" Alice+tag@Example.Test ").unwrap(),
            "alice+tag@example.test"
        );
        assert_eq!(
            normalize_email("a.b@example.test").unwrap(),
            "a.b@example.test"
        );
        for input in [
            "",
            "not an address",
            "Name <alice@example.test>",
            "alice@example.test\r\nBcc: attacker@example.test",
        ] {
            assert_eq!(normalize_email(input), Err(AccountError::InvalidEmail));
        }
    }

    #[test]
    fn handles_are_bounded_ascii_and_not_silently_trimmed() {
        for valid in ["abc", "Alice_123", &"x".repeat(30)] {
            assert!(validate_handle(valid).is_ok());
        }
        for invalid in ["ab", &"x".repeat(31), " alice ", "na-me", "用户名称"] {
            assert_eq!(validate_handle(invalid), Err(AccountError::InvalidHandle));
        }
    }
}
