use sqlx::PgPool;
use std::{
    collections::HashSet,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::task::JoinSet;
use twitter_clone_api::{
    database::MIGRATOR,
    jobs::{self, DeliveryFailure, Intent, JobError, MAX_ATTEMPTS, MAX_BATCH, RetryState},
};
use uuid::Uuid;

mod support;

async fn test_pool() -> PgPool {
    let pool = support::fresh_pool().await;
    MIGRATOR.run(&pool).await.unwrap();
    pool
}

async fn enqueue_key(pool: &PgPool) -> Uuid {
    jobs::enqueue(
        &mut pool.acquire().await.unwrap(),
        &Intent::RevokeKey {
            key_id: format!("key_{}", Uuid::new_v4().simple()),
        },
    )
    .await
    .unwrap()
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn concurrent_workers_claim_each_intent_once_and_respect_batch_bounds() {
    let pool = test_pool().await;
    for _ in 0..20 {
        enqueue_key(&pool).await;
    }
    assert!(matches!(
        jobs::claim_batch(&pool, 0).await,
        Err(JobError::InvalidBatch)
    ));
    assert!(matches!(
        jobs::claim_batch(&pool, MAX_BATCH + 1).await,
        Err(JobError::InvalidBatch)
    ));
    let mut workers = JoinSet::new();
    for _ in 0..8 {
        let pool = pool.clone();
        workers.spawn(async move { jobs::claim_batch(&pool, 4).await.unwrap() });
    }
    let mut claimed = HashSet::new();
    while let Some(batch) = workers.join_next().await {
        let batch = batch.unwrap();
        assert!(batch.len() <= 4);
        for claim in batch {
            assert!(claimed.insert(claim.id), "intent was claimed twice");
            assert_eq!(claim.attempts, 1);
            jobs::complete(&mut pool.acquire().await.unwrap(), &claim)
                .await
                .unwrap();
        }
    }
    assert_eq!(claimed.len(), 20);
    assert!(jobs::claim_batch(&pool, 4).await.unwrap().is_empty());
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn expired_claims_recover_and_old_workers_cannot_acknowledge_or_retry() {
    let pool = test_pool().await;
    let id = enqueue_key(&pool).await;
    let old = jobs::claim_batch(&pool, 1).await.unwrap().remove(0);
    assert!(jobs::claim_batch(&pool, 1).await.unwrap().is_empty());
    sqlx::query("UPDATE delivery_jobs SET lease_until = clock_timestamp() - interval '1 second' WHERE id = $1")
        .bind(id).execute(&pool).await.unwrap();
    let current = jobs::claim_batch(&pool, 1).await.unwrap().remove(0);
    assert_eq!(current.id, id);
    assert_ne!(current.claim_id, old.claim_id);
    assert_eq!(current.attempts, 2);
    let mut connection = pool.acquire().await.unwrap();
    assert_eq!(
        jobs::lock_claim(&mut connection, &old).await,
        Err(JobError::LostLease)
    );
    assert_eq!(
        jobs::complete(&mut connection, &old).await,
        Err(JobError::LostLease)
    );
    assert_eq!(
        jobs::retry(&mut connection, &old).await,
        Err(JobError::LostLease)
    );
    jobs::lock_claim(&mut connection, &current).await.unwrap();
    jobs::complete(&mut connection, &current).await.unwrap();
    drop(connection);
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn retry_budget_is_durable_and_exhausted_intents_are_retained() {
    let pool = test_pool().await;
    let id = enqueue_key(&pool).await;
    let claim = jobs::claim_batch(&pool, 1).await.unwrap().remove(0);
    assert_eq!(
        jobs::retry(&mut pool.acquire().await.unwrap(), &claim)
            .await
            .unwrap(),
        RetryState::Scheduled
    );
    assert!(jobs::claim_batch(&pool, 1).await.unwrap().is_empty());
    sqlx::query(
        "UPDATE delivery_jobs SET available_at = now() - interval '1 minute', attempts = $2 WHERE id = $1",
    ).bind(id).bind((MAX_ATTEMPTS - 1) as i32).execute(&pool).await.unwrap();
    let final_claim = jobs::claim_batch(&pool, 1).await.unwrap().remove(0);
    assert_eq!(final_claim.attempts, MAX_ATTEMPTS);
    assert_eq!(
        jobs::retry(&mut pool.acquire().await.unwrap(), &final_claim)
            .await
            .unwrap(),
        RetryState::Exhausted
    );
    sqlx::query(
        "UPDATE delivery_jobs SET available_at = now() - interval '1 minute' WHERE id = $1",
    )
    .bind(id)
    .execute(&pool)
    .await
    .unwrap();
    assert!(jobs::claim_batch(&pool, 1).await.unwrap().is_empty());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM delivery_jobs WHERE id = $1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn enqueue_shares_business_transactions_and_stores_only_typed_intents() {
    let pool = test_pool().await;
    let user = Uuid::new_v4();
    let mut tx = pool.begin().await.unwrap();
    sqlx::query(
        "INSERT INTO users (id, email_normalized, handle, password_hash, display_name) \
         VALUES ($1, 'fixture@example.test', 'fixture', 'test-fixture-not-a-password-hash', 'Fixture')",
    ).bind(user).execute(&mut *tx).await.unwrap();
    jobs::enqueue(&mut tx, &Intent::VerificationEmail { user_id: user })
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    assert!(jobs::claim_batch(&pool, 4).await.unwrap().is_empty());
    let mut tx = pool.begin().await.unwrap();
    sqlx::query(
        "INSERT INTO users (id, email_normalized, handle, password_hash, display_name) \
         VALUES ($1, 'fixture@example.test', 'fixture', 'test-fixture-not-a-password-hash', 'Fixture')",
    ).bind(user).execute(&mut *tx).await.unwrap();
    jobs::enqueue(&mut tx, &Intent::VerificationEmail { user_id: user })
        .await
        .unwrap();
    jobs::enqueue(&mut tx, &Intent::PasswordResetEmail { user_id: user })
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let claims = jobs::claim_batch(&pool, 4).await.unwrap();
    assert_eq!(claims.len(), 2);
    assert!(
        claims
            .iter()
            .any(|c| c.intent == Intent::VerificationEmail { user_id: user })
    );
    assert!(
        claims
            .iter()
            .any(|c| c.intent == Intent::PasswordResetEmail { user_id: user })
    );
    let invalid = Intent::RevokeKey {
        key_id: "full-key-secrets-do-not-belong-here".into(),
    };
    assert_eq!(
        jobs::enqueue(&mut pool.acquire().await.unwrap(), &invalid).await,
        Err(JobError::InvalidIntent),
    );
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn batch_delivery_retries_failure_then_completes_without_losing_intent() {
    let pool = test_pool().await;
    enqueue_key(&pool).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let outcome = jobs::run_batch(&pool, {
        let calls = calls.clone();
        move |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Err(DeliveryFailure) }
        }
    })
    .await
    .unwrap();
    assert_eq!(outcome.claimed, 1);
    assert_eq!(outcome.retried, 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    sqlx::query("UPDATE delivery_jobs SET available_at = now() - interval '1 minute'")
        .execute(&pool)
        .await
        .unwrap();
    let outcome = jobs::run_batch(&pool, |_| async { Ok(()) }).await.unwrap();
    assert_eq!(outcome.completed, 1);
    assert!(jobs::claim_batch(&pool, 4).await.unwrap().is_empty());
    pool.close().await;
}
