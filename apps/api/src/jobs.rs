use sqlx::{PgConnection, PgPool};
use std::{error::Error, fmt, future::Future, time::Duration};
use tokio::task::JoinSet;
use uuid::Uuid;

pub const MAX_ATTEMPTS: u32 = 8;
pub const MAX_BATCH: u32 = 16;
const LEASE_SECONDS: i64 = 30;
const WORKER_BATCH: u32 = 4;

#[derive(Clone, Copy, Debug)]
pub enum Kind {
    VerificationEmail,
    PasswordResetEmail,
    RevokeKey,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Self::VerificationEmail => "verification_email",
            Self::PasswordResetEmail => "password_reset_email",
            Self::RevokeKey => "unkey_revoke",
        }
    }
}

const ALL_KINDS: &[Kind] = &[
    Kind::VerificationEmail,
    Kind::PasswordResetEmail,
    Kind::RevokeKey,
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    VerificationEmail { user_id: Uuid },
    PasswordResetEmail { user_id: Uuid },
    RevokeKey { key_id: String },
}

#[derive(Clone, Debug)]
pub struct Claim {
    pub id: Uuid,
    pub claim_id: Uuid,
    pub attempts: u32,
    pub intent: Intent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobError {
    InvalidIntent,
    InvalidBatch,
    Unavailable,
    LostLease,
    WorkerFailed,
}

impl fmt::Display for JobError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidIntent => "Delivery intent is invalid",
            Self::InvalidBatch => "Delivery batch size is invalid",
            Self::Unavailable => "Delivery job storage is unavailable",
            Self::LostLease => "Delivery job lease is no longer owned",
            Self::WorkerFailed => "Delivery worker did not complete",
        })
    }
}

impl Error for JobError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetryState {
    Scheduled,
    Exhausted,
}

#[derive(Clone, Copy, Debug)]
pub struct DeliveryFailure;

#[derive(Default, Debug)]
pub struct BatchOutcome {
    pub claimed: u32,
    pub completed: u32,
    pub retried: u32,
    pub exhausted: u32,
    pub lost_leases: u32,
}

/// Use the caller's transaction when enqueueing alongside account/key changes.
pub async fn enqueue(connection: &mut PgConnection, intent: &Intent) -> Result<Uuid, JobError> {
    let (kind, user_id, key_id) = match intent {
        Intent::VerificationEmail { user_id } => ("verification_email", Some(*user_id), None),
        Intent::PasswordResetEmail { user_id } => ("password_reset_email", Some(*user_id), None),
        Intent::RevokeKey { key_id } => {
            if !key_id.starts_with("key_")
                || key_id.len() <= 4
                || key_id.len() > 255
                || !key_id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
            {
                return Err(JobError::InvalidIntent);
            }
            ("unkey_revoke", None, Some(key_id.as_str()))
        }
    };
    database_operation(
        sqlx::query_scalar(
            "INSERT INTO delivery_jobs (id, kind, user_id, key_id) \
             VALUES ($1, $2, $3, $4) RETURNING id",
        )
        .bind(Uuid::new_v4())
        .bind(kind)
        .bind(user_id)
        .bind(key_id)
        .fetch_one(connection),
    )
    .await
}

#[derive(sqlx::FromRow)]
struct StoredClaim {
    id: Uuid,
    claim_id: Uuid,
    kind: String,
    user_id: Option<Uuid>,
    key_id: Option<String>,
    attempts: i32,
}

pub async fn claim_batch(pool: &PgPool, batch_size: u32) -> Result<Vec<Claim>, JobError> {
    claim_batch_for(pool, batch_size, ALL_KINDS).await
}

pub async fn claim_batch_for(
    pool: &PgPool,
    batch_size: u32,
    kinds: &[Kind],
) -> Result<Vec<Claim>, JobError> {
    if batch_size == 0 || batch_size > MAX_BATCH || kinds.is_empty() {
        return Err(JobError::InvalidBatch);
    }
    let allowed: Vec<&str> = kinds.iter().map(|kind| kind.as_str()).collect();
    let rows: Vec<StoredClaim> = database_operation(
        sqlx::query_as(
            "WITH candidates AS ( \
                 SELECT id FROM delivery_jobs \
                 WHERE attempts < $2 AND available_at <= clock_timestamp() AND kind = ANY($5) \
                     AND (lease_until IS NULL OR lease_until <= clock_timestamp()) \
                 ORDER BY available_at, id LIMIT $1 FOR UPDATE SKIP LOCKED \
             ) \
             UPDATE delivery_jobs AS job \
             SET attempts = job.attempts + 1, claim_id = $3, \
                 lease_until = clock_timestamp() + $4::bigint * interval '1 second' \
             FROM candidates WHERE job.id = candidates.id \
             RETURNING job.id, job.claim_id, job.kind, job.user_id, job.key_id, job.attempts",
        )
        .bind(i64::from(batch_size))
        .bind(MAX_ATTEMPTS as i32)
        .bind(Uuid::new_v4())
        .bind(LEASE_SECONDS)
        .bind(allowed)
        .fetch_all(pool),
    )
    .await?;
    rows.into_iter()
        .map(|row| {
            let intent = match (row.kind.as_str(), row.user_id, row.key_id) {
                ("verification_email", Some(user_id), None) => {
                    Intent::VerificationEmail { user_id }
                }
                ("password_reset_email", Some(user_id), None) => {
                    Intent::PasswordResetEmail { user_id }
                }
                ("unkey_revoke", _, Some(key_id)) => Intent::RevokeKey { key_id },
                _ => return Err(JobError::InvalidIntent),
            };
            Ok(Claim {
                id: row.id,
                claim_id: row.claim_id,
                attempts: row.attempts as u32,
                intent,
            })
        })
        .collect()
}

/// Lock and validate the lease in the same transaction as token/key metadata work.
pub async fn lock_claim(connection: &mut PgConnection, claim: &Claim) -> Result<(), JobError> {
    let owned: Option<Uuid> = database_operation(
        sqlx::query_scalar(
            "SELECT id FROM delivery_jobs WHERE id = $1 AND claim_id = $2 \
             AND lease_until > clock_timestamp() FOR UPDATE",
        )
        .bind(claim.id)
        .bind(claim.claim_id)
        .fetch_optional(connection),
    )
    .await?;
    owned.map(|_| ()).ok_or(JobError::LostLease)
}

pub async fn complete(connection: &mut PgConnection, claim: &Claim) -> Result<(), JobError> {
    let result = database_operation(
        sqlx::query(
            "DELETE FROM delivery_jobs WHERE id = $1 AND claim_id = $2 \
             AND lease_until > clock_timestamp()",
        )
        .bind(claim.id)
        .bind(claim.claim_id)
        .execute(connection),
    )
    .await?;
    if result.rows_affected() == 1 {
        Ok(())
    } else {
        Err(JobError::LostLease)
    }
}

pub async fn retry(connection: &mut PgConnection, claim: &Claim) -> Result<RetryState, JobError> {
    let delay = 5_i64 << claim.attempts.saturating_sub(1).min(7);
    let result = database_operation(
        sqlx::query(
            "UPDATE delivery_jobs SET claim_id = NULL, lease_until = NULL, \
                 available_at = clock_timestamp() + $3::bigint * interval '1 second' \
             WHERE id = $1 AND claim_id = $2 AND lease_until > clock_timestamp()",
        )
        .bind(claim.id)
        .bind(claim.claim_id)
        .bind(delay)
        .execute(connection),
    )
    .await?;
    if result.rows_affected() != 1 {
        return Err(JobError::LostLease);
    }
    Ok(if claim.attempts >= MAX_ATTEMPTS {
        RetryState::Exhausted
    } else {
        RetryState::Scheduled
    })
}

/// Run at most four deliveries concurrently, each within a 30-second lease.
///
/// Drivers are supplied by email/key integrations, not a runtime mock mode.
/// Delivery is at least once: drivers must tolerate replay after a crash.
pub async fn run_batch<F, Fut>(pool: &PgPool, deliver: F) -> Result<BatchOutcome, JobError>
where
    F: Fn(Claim) -> Fut + Clone + Send + 'static,
    Fut: Future<Output = Result<(), DeliveryFailure>> + Send + 'static,
{
    run_batch_for(pool, ALL_KINDS, deliver).await
}

pub async fn run_batch_for<F, Fut>(
    pool: &PgPool,
    kinds: &[Kind],
    deliver: F,
) -> Result<BatchOutcome, JobError>
where
    F: Fn(Claim) -> Fut + Clone + Send + 'static,
    Fut: Future<Output = Result<(), DeliveryFailure>> + Send + 'static,
{
    let claims = claim_batch_for(pool, WORKER_BATCH, kinds).await?;
    let mut outcome = BatchOutcome {
        claimed: claims.len() as u32,
        ..BatchOutcome::default()
    };
    let mut workers = JoinSet::new();
    for claim in claims {
        let pool = pool.clone();
        let deliver = deliver.clone();
        workers.spawn(async move {
            let delivered = tokio::time::timeout(Duration::from_secs(10), deliver(claim.clone()))
                .await
                .is_ok_and(|result| result.is_ok());
            let mut connection = database_operation(pool.acquire()).await?;
            if delivered {
                complete(&mut connection, &claim).await.map(|_| None)
            } else {
                retry(&mut connection, &claim).await.map(Some)
            }
        });
    }
    while let Some(result) = workers.join_next().await {
        match result.map_err(|_| JobError::WorkerFailed)? {
            Ok(None) => outcome.completed += 1,
            Ok(Some(RetryState::Scheduled)) => outcome.retried += 1,
            Ok(Some(RetryState::Exhausted)) => outcome.exhausted += 1,
            Err(JobError::LostLease) => outcome.lost_leases += 1,
            Err(error) => return Err(error),
        }
    }
    Ok(outcome)
}

async fn database_operation<T>(
    operation: impl Future<Output = Result<T, sqlx::Error>>,
) -> Result<T, JobError> {
    tokio::time::timeout(Duration::from_secs(2), operation)
        .await
        .map_err(|_| JobError::Unavailable)?
        .map_err(|_| JobError::Unavailable)
}
