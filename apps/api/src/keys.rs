use crate::{
    accounts::ActiveSession,
    config::Config,
    http::ApiError,
    jobs::{self, DeliveryFailure, Intent, Kind},
    rate_limit::{RateLimitError, RateLimiter, Rule},
    unkey::{CreatedKey, KeyCreate, Management, UnkeyError},
};
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{PgConnection, PgPool};
use std::time::Duration;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyError {
    Unauthenticated,
    Unverified,
    RecentPasswordRequired,
    NotFound,
    IntegrationUnavailable,
    Provider(UnkeyError),
    Unavailable,
    Limited(RateLimitError),
}

impl IntoResponse for KeyError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            Self::Unauthenticated => (
                StatusCode::UNAUTHORIZED,
                "authentication_required",
                "Log in to continue.",
            ),
            Self::Unverified => (
                StatusCode::FORBIDDEN,
                "verification_required",
                "Verify your email to continue.",
            ),
            Self::RecentPasswordRequired => (
                StatusCode::FORBIDDEN,
                "recent_password_required",
                "Confirm your password before creating a key.",
            ),
            Self::NotFound => (
                StatusCode::NOT_FOUND,
                "not_found",
                "The resource was not found.",
            ),
            Self::IntegrationUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "integration_unavailable",
                "API key integration requires setup.",
            ),
            Self::Provider(UnkeyError::InvalidInput) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_key_request",
                "Choose a name, supported permissions, and an expiry within 365 days.",
            ),
            Self::Provider(UnkeyError::AmbiguousCreate) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "key_creation_uncertain",
                "Key creation status is uncertain. Reconcile before retrying.",
            ),
            Self::Unavailable | Self::Provider(_) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "service_unavailable",
                "Key management is temporarily unavailable.",
            ),
            Self::Limited(error) => return error.into_response(),
        };
        ApiError {
            status,
            code,
            message,
        }
        .into_response()
    }
}

#[derive(Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct KeyMetadata {
    pub key_id: String,
    pub name: String,
    pub masked_hint: String,
    pub permissions: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub provider_revoked_at: Option<DateTime<Utc>>,
    pub status: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyList {
    pub integration_available: bool,
    pub items: Vec<KeyMetadata>,
}

// No Debug/Serialize: only the browser adapter may explicitly reveal the secret.
pub struct IssuedKey {
    pub metadata: KeyMetadata,
    pub grant: CreatedKey,
}

#[derive(Clone)]
pub struct Keys {
    pool: PgPool,
    limiter: RateLimiter,
    management: Option<Management>,
}

const METADATA: &str = "SELECT key_id, label AS name, masked_hint, permissions, \
    created_at, expires_at, revoked_at, provider_revoked_at, \
    CASE WHEN revoked_at IS NOT NULL AND provider_revoked_at IS NULL THEN 'revoked_pending_cleanup' \
         WHEN revoked_at IS NOT NULL THEN 'revoked' \
         WHEN expires_at <= clock_timestamp() THEN 'expired' ELSE 'active' END AS status \
    FROM api_keys";

impl Keys {
    pub fn new(pool: PgPool, config: &Config) -> Result<Self, KeyError> {
        Ok(Self {
            limiter: RateLimiter::new(pool.clone(), &config.rate_limit_secret)
                .map_err(KeyError::Limited)?,
            management: config
                .unkey
                .as_ref()
                .map(Management::new)
                .transpose()
                .map_err(KeyError::Provider)?,
            pool,
        })
    }

    async fn throttle(&self, session: &ActiveSession) -> Result<(), KeyError> {
        self.limiter
            .check(
                "keys:management:user",
                session.identity.id.as_bytes(),
                Rule::new(20, 60).expect("positive server-owned rule"),
            )
            .await
            .map_err(KeyError::Limited)
    }

    pub async fn list(&self, session: &ActiveSession) -> Result<KeyList, KeyError> {
        self.throttle(session).await?;
        let mut tx = self.pool.begin().await.map_err(|_| KeyError::Unavailable)?;
        authenticate(&mut tx, session, false).await?;
        let query = format!("{METADATA} WHERE user_id = $1 ORDER BY created_at DESC, key_id DESC");
        let items = sqlx::query_as(sqlx::AssertSqlSafe(query))
            .bind(session.identity.id)
            .fetch_all(&mut *tx)
            .await
            .map_err(|_| KeyError::Unavailable)?;
        tx.commit().await.map_err(|_| KeyError::Unavailable)?;
        Ok(KeyList {
            integration_available: self.management.is_some(),
            items,
        })
    }

    pub async fn create(
        &self,
        session: &ActiveSession,
        input: KeyCreate,
    ) -> Result<IssuedKey, KeyError> {
        self.throttle(session).await?;
        input.validate().map_err(KeyError::Provider)?;
        let mut tx = self.pool.begin().await.map_err(|_| KeyError::Unavailable)?;
        authenticate(&mut tx, session, true).await?;
        let management = self
            .management
            .as_ref()
            .ok_or(KeyError::IntegrationUnavailable)?;
        let now: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut *tx)
            .await
            .map_err(|_| KeyError::Unavailable)?;
        let grant = management
            .create_key(session.identity.id, &input, now)
            .await
            .map_err(KeyError::Provider)?;
        // Serialize names through the same enum contract as the provider request.
        let permissions: Vec<String> = serde_json::from_value(
            serde_json::to_value(&input.permissions)
                .expect("permission enum serializes to strings"),
        )
        .expect("permission enum serializes to strings");
        let metadata = async {
            sqlx::query(
                "INSERT INTO api_keys (key_id, user_id, keyspace_id, label, masked_hint, permissions, expires_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
            ).bind(&grant.key_id).bind(session.identity.id).bind(management.keyspace_id())
                .bind(&input.name).bind(grant.masked_hint()).bind(permissions).bind(grant.expires_at)
                .execute(&mut *tx).await?;
            let query = format!("{METADATA} WHERE key_id = $1 AND user_id = $2");
            sqlx::query_as::<_, KeyMetadata>(sqlx::AssertSqlSafe(query))
                .bind(&grant.key_id).bind(session.identity.id).fetch_one(&mut *tx).await
        }.await;
        let metadata = match metadata {
            Ok(metadata) => metadata,
            Err(_) => {
                let _ = tx.rollback().await;
                self.failed_issuance(management, session.identity.id, &grant.key_id)
                    .await;
                return Err(KeyError::Unavailable);
            }
        };
        if tx.commit().await.is_err() {
            // An uncertain DB commit must not return the secret either. Revoke
            // any committed record and queue cleanup in a fresh transaction.
            self.failed_issuance(management, session.identity.id, &grant.key_id)
                .await;
            return Err(KeyError::Unavailable);
        }
        Ok(IssuedKey { metadata, grant })
    }

    async fn failed_issuance(&self, management: &Management, owner: Uuid, key_id: &str) {
        let queued = async {
            let mut tx = self.pool.begin().await.map_err(|_| KeyError::Unavailable)?;
            sqlx::query("SELECT id FROM users WHERE id = $1 FOR UPDATE")
                .bind(owner)
                .fetch_one(&mut *tx)
                .await
                .map_err(|_| KeyError::Unavailable)?;
            sqlx::query(
                "UPDATE api_keys SET revoked_at = coalesce(revoked_at, clock_timestamp()) \
                WHERE key_id = $1 AND user_id = $2",
            )
            .bind(key_id)
            .bind(owner)
            .execute(&mut *tx)
            .await
            .map_err(|_| KeyError::Unavailable)?;
            jobs::enqueue(
                &mut tx,
                &Intent::RevokeKey {
                    key_id: key_id.into(),
                },
            )
            .await
            .map_err(|_| KeyError::Unavailable)?;
            tx.commit().await.map_err(|_| KeyError::Unavailable)
        }
        .await;
        if queued.is_err() {
            // A total storage outage cannot persist a cleanup intent. Best
            // effort deletion is safe; unknown keys still deny locally. An
            // ambiguous outcome remains an operator reconciliation case.
            let _ = management.delete_key(key_id).await;
        }
    }

    pub async fn revoke(&self, session: &ActiveSession, key_id: &str) -> Result<(), KeyError> {
        self.throttle(session).await?;
        let mut tx = self.pool.begin().await.map_err(|_| KeyError::Unavailable)?;
        authenticate(&mut tx, session, false).await?;
        let revoked: Option<bool> = sqlx::query_scalar(
            "SELECT revoked_at IS NOT NULL FROM api_keys WHERE key_id = $1 AND user_id = $2 FOR UPDATE",
        ).bind(key_id).bind(session.identity.id).fetch_optional(&mut *tx).await.map_err(|_| KeyError::Unavailable)?;
        match revoked {
            None => return Err(KeyError::NotFound),
            Some(true) => {}
            Some(false) => {
                sqlx::query("UPDATE api_keys SET revoked_at = clock_timestamp() WHERE key_id = $1")
                    .bind(key_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|_| KeyError::Unavailable)?;
                jobs::enqueue(
                    &mut tx,
                    &Intent::RevokeKey {
                        key_id: key_id.into(),
                    },
                )
                .await
                .map_err(|_| KeyError::Unavailable)?;
            }
        }
        tx.commit().await.map_err(|_| KeyError::Unavailable)
    }

    async fn cleanup_batch(&self) -> Result<jobs::BatchOutcome, jobs::JobError> {
        let Some(management) = self.management.clone() else {
            return Ok(jobs::BatchOutcome::default());
        };
        let pool = self.pool.clone();
        jobs::run_batch_for(&self.pool, &[Kind::RevokeKey], move |claim| {
            let management = management.clone();
            let pool = pool.clone();
            async move {
                let Intent::RevokeKey { ref key_id } = claim.intent else {
                    return Err(DeliveryFailure);
                };
                management
                    .delete_key(key_id)
                    .await
                    .map_err(|_| DeliveryFailure)?;
                let mut tx = pool.begin().await.map_err(|_| DeliveryFailure)?;
                jobs::lock_claim(&mut tx, &claim)
                    .await
                    .map_err(|_| DeliveryFailure)?;
                sqlx::query(
                    "UPDATE api_keys SET provider_revoked_at = clock_timestamp() \
                    WHERE key_id = $1 AND revoked_at IS NOT NULL",
                )
                .bind(key_id)
                .execute(&mut *tx)
                .await
                .map_err(|_| DeliveryFailure)?;
                tx.commit().await.map_err(|_| DeliveryFailure)
            }
        })
        .await
    }

    pub async fn run(self) {
        if self.management.is_none() {
            return;
        }
        let mut ticks = tokio::time::interval(Duration::from_secs(2));
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticks.tick().await;
            match self.cleanup_batch().await {
                Ok(outcome) if outcome.exhausted > 0 => {
                    eprintln!("Provider cleanup exhausted its retry budget")
                }
                Ok(outcome) if outcome.retried > 0 => eprintln!("Provider cleanup will be retried"),
                Err(_) => eprintln!("Provider cleanup queue is temporarily unavailable"),
                _ => {}
            }
        }
    }
}

async fn authenticate(
    connection: &mut PgConnection,
    session: &ActiveSession,
    recent: bool,
) -> Result<(), KeyError> {
    // Account first, then fresh session query. Holding the account lock across
    // provider creation and local commit serializes issuance with password reset.
    let verified: Option<bool> =
        sqlx::query_scalar("SELECT verified_at IS NOT NULL FROM users WHERE id = $1 FOR UPDATE")
            .bind(session.identity.id)
            .fetch_optional(&mut *connection)
            .await
            .map_err(|_| KeyError::Unavailable)?;
    match verified {
        None => return Err(KeyError::Unauthenticated),
        Some(false) => return Err(KeyError::Unverified),
        Some(true) => {}
    }
    let live: Option<bool> = sqlx::query_scalar(
        "SELECT password_authenticated_at > clock_timestamp() - interval '5 minutes' \
         FROM sessions WHERE token_digest = $1 AND user_id = $2 AND revoked_at IS NULL \
         AND expires_at > clock_timestamp() AND last_used_at > clock_timestamp() - interval '24 hours'",
    ).bind(session.binding()).bind(session.identity.id).fetch_optional(connection).await.map_err(|_| KeyError::Unavailable)?;
    match live {
        None => Err(KeyError::Unauthenticated),
        Some(false) if recent => Err(KeyError::RecentPasswordRequired),
        Some(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests;
