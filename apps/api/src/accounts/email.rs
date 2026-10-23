use super::{AccountError, Accounts, normalize_email};
use crate::{
    config::{Config, Environment},
    jobs::{self, Claim, DeliveryFailure, Intent, Kind},
    tokens::{self, BearerToken},
};
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{Mailbox, header::ContentType},
    transport::smtp::authentication::Credentials,
};
use sqlx::{PgPool, Row};
use std::time::Duration;
use url::Url;
use uuid::Uuid;

const EMAIL_KINDS: &[Kind] = &[Kind::VerificationEmail, Kind::PasswordResetEmail];

#[derive(Clone, Copy)]
enum Purpose {
    Verification,
    Reset,
}

impl Purpose {
    fn database_name(self) -> &'static str {
        match self {
            Self::Verification => "email_verification",
            Self::Reset => "password_reset",
        }
    }
    fn seconds(self) -> i64 {
        match self {
            Self::Verification => 24 * 60 * 60,
            Self::Reset => 30 * 60,
        }
    }
    fn path(self) -> &'static str {
        match self {
            Self::Verification => "/verify-email",
            Self::Reset => "/reset-password",
        }
    }
    fn subject(self) -> &'static str {
        match self {
            Self::Verification => "Verify your email",
            Self::Reset => "Reset your password",
        }
    }
}

// No Debug: the transport/configuration and transient message contain secrets.
#[derive(Clone)]
pub struct EmailDelivery {
    pool: PgPool,
    origin: Url,
    from: Mailbox,
    transport: AsyncSmtpTransport<Tokio1Executor>,
}

impl EmailDelivery {
    pub fn new(pool: PgPool, config: &Config) -> Result<Self, AccountError> {
        let smtp = &config.smtp;
        let mut builder = if smtp.tls {
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&smtp.host)
                .map_err(|_| AccountError::Unavailable)?
        } else if config.environment == Environment::Development {
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&smtp.host)
        } else {
            return Err(AccountError::Unavailable);
        };
        if let (Some(username), Some(password)) = (&smtp.username, &smtp.password) {
            builder = builder.credentials(Credentials::new(username.clone(), password.clone()));
        }
        let from = Mailbox::new(
            Some("TwitterCloneAgent".into()),
            smtp.from_address
                .parse()
                .map_err(|_| AccountError::Unavailable)?,
        );
        Ok(Self {
            pool,
            origin: config.app_origin.clone(),
            from,
            transport: builder
                .port(smtp.port)
                .timeout(Some(Duration::from_secs(5)))
                .build(),
        })
    }

    pub async fn deliver(&self, claim: &Claim) -> Result<(), AccountError> {
        let (user_id, purpose) = match claim.intent {
            Intent::VerificationEmail { user_id } => (user_id, Purpose::Verification),
            Intent::PasswordResetEmail { user_id } => (user_id, Purpose::Reset),
            Intent::RevokeKey { .. } => return Err(AccountError::Unavailable),
        };
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| AccountError::Unavailable)?;
        // All account mutations lock the account before its jobs/tokens/sessions.
        // A claim that expired while waiting is rechecked before token changes.
        let account = sqlx::query(
            "SELECT email_normalized, verified_at IS NOT NULL AS verified FROM users WHERE id = $1 FOR UPDATE",
        ).bind(user_id).fetch_one(&mut *tx).await.map_err(|_| AccountError::Unavailable)?;
        jobs::lock_claim(&mut tx, claim)
            .await
            .map_err(|_| AccountError::Unavailable)?;
        if matches!(purpose, Purpose::Verification) && account.get::<bool, _>("verified") {
            tx.commit().await.map_err(|_| AccountError::Unavailable)?;
            return Ok(());
        }
        let recipient: String = account.get("email_normalized");
        let token = BearerToken::generate()?;
        sqlx::query(
            "UPDATE account_tokens SET consumed_at = clock_timestamp() \
             WHERE user_id = $1 AND purpose = $2 AND consumed_at IS NULL",
        )
        .bind(user_id)
        .bind(purpose.database_name())
        .execute(&mut *tx)
        .await
        .map_err(|_| AccountError::Unavailable)?;
        sqlx::query(
            "INSERT INTO account_tokens (token_digest, user_id, purpose, expires_at) \
             VALUES ($1, $2, $3, now() + $4::bigint * interval '1 second')",
        )
        .bind(token.digest())
        .bind(user_id)
        .bind(purpose.database_name())
        .bind(purpose.seconds())
        .execute(&mut *tx)
        .await
        .map_err(|_| AccountError::Unavailable)?;
        tx.commit().await.map_err(|_| AccountError::Unavailable)?;

        let mut link = self
            .origin
            .join(purpose.path())
            .map_err(|_| AccountError::Unavailable)?;
        link.query_pairs_mut()
            .append_pair("token", token.expose_secret());
        let message = Message::builder()
            .from(self.from.clone())
            .to(Mailbox::new(
                None,
                recipient.parse().map_err(|_| AccountError::Unavailable)?,
            ))
            .subject(purpose.subject())
            .header(ContentType::TEXT_PLAIN)
            .body(format!(
                "{} for TwitterCloneAgent:\n\n{}\n\n\
                 Open this page and submit the form to finish. \
                 If you did not request this email, you can ignore it.\n",
                purpose.subject(),
                link,
            ))
            .map_err(|_| AccountError::Unavailable)?;
        self.transport
            .send(message)
            .await
            .map_err(|_| AccountError::Unavailable)?;
        Ok(())
    }

    pub async fn run(self) {
        let mut ticks = tokio::time::interval(Duration::from_secs(2));
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticks.tick().await;
            let worker = self.clone();
            let result = jobs::run_batch_for(&self.pool, EMAIL_KINDS, move |claim| {
                let worker = worker.clone();
                async move { worker.deliver(&claim).await.map_err(|_| DeliveryFailure) }
            })
            .await;
            match result {
                Ok(result) if result.exhausted > 0 => {
                    eprintln!("Email delivery exhausted its retry budget")
                }
                Ok(result) if result.retried > 0 => eprintln!("Email delivery will be retried"),
                Err(_) => eprintln!("Email delivery queue is temporarily unavailable"),
                _ => {}
            }
        }
    }
}

impl Accounts {
    pub async fn request_verification(&self, user_id: Uuid) -> Result<(), AccountError> {
        sqlx::query(
            "INSERT INTO delivery_jobs (id, kind, user_id) \
             SELECT $1, 'verification_email', id FROM users WHERE id = $2 AND verified_at IS NULL",
        )
        .bind(Uuid::new_v4())
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|_| AccountError::Unavailable)?;
        Ok(())
    }

    pub async fn request_password_reset(&self, email: &str) -> Result<(), AccountError> {
        let email = normalize_email(email)?;
        sqlx::query(
            "INSERT INTO delivery_jobs (id, kind, user_id) \
             SELECT $1, 'password_reset_email', id FROM users WHERE email_normalized = $2",
        )
        .bind(Uuid::new_v4())
        .bind(email)
        .execute(&self.pool)
        .await
        .map_err(|_| AccountError::Unavailable)?;
        Ok(())
    }

    pub async fn verify_email(&self, token: &str) -> Result<(), AccountError> {
        let digest = tokens::digest(token).ok_or(AccountError::InvalidToken)?;
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| AccountError::Unavailable)?;
        let user_id: Option<Uuid> = sqlx::query_scalar(
            "SELECT user_id FROM account_tokens WHERE token_digest = $1 \
             AND purpose = 'email_verification' AND consumed_at IS NULL \
             AND expires_at > clock_timestamp()",
        )
        .bind(&digest)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| AccountError::Unavailable)?;
        let user_id = user_id.ok_or(AccountError::InvalidToken)?;
        // Match delivery/recovery lock ordering: account first, then its tokens.
        sqlx::query("SELECT id FROM users WHERE id = $1 FOR UPDATE")
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|_| AccountError::Unavailable)?;
        let consumed = sqlx::query(
            "UPDATE account_tokens SET consumed_at = clock_timestamp() \
             WHERE token_digest = $1 AND purpose = 'email_verification' \
                 AND user_id = $2 AND consumed_at IS NULL AND expires_at > clock_timestamp()",
        )
        .bind(digest)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(|_| AccountError::Unavailable)?;
        if consumed.rows_affected() != 1 {
            return Err(AccountError::InvalidToken);
        }
        sqlx::query(
            "UPDATE users SET verified_at = coalesce(verified_at, clock_timestamp()) WHERE id = $1",
        )
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(|_| AccountError::Unavailable)?;
        tx.commit().await.map_err(|_| AccountError::Unavailable)
    }
}
