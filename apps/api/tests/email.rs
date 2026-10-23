use sqlx::PgPool;
use std::collections::HashMap;
use twitter_clone_api::{
    accounts::{AccountError, Accounts, EmailDelivery},
    config::Config,
    database::MIGRATOR,
    jobs::{self, DeliveryFailure, Intent, Kind},
    tokens::BearerToken,
};
use uuid::Uuid;

mod support;

const PASSWORD: &str = "isolated email fixture password";

async fn setup() -> (PgPool, Accounts, Uuid) {
    let pool = support::fresh_pool().await;
    MIGRATOR.run(&pool).await.unwrap();
    let accounts = Accounts::new(pool.clone()).await.unwrap();
    accounts
        .register("fixture@example.test", "fixture", PASSWORD)
        .await
        .unwrap();
    let identity = accounts
        .authenticate("fixture@example.test", PASSWORD)
        .await
        .unwrap();
    (pool, accounts, identity.id)
}

async fn fixture_token(pool: &PgPool, user: Uuid, purpose: &str, expired: bool) -> BearerToken {
    let token = BearerToken::generate().unwrap();
    sqlx::query(
        "UPDATE account_tokens SET consumed_at = now() \
         WHERE user_id = $1 AND purpose = $2 AND consumed_at IS NULL",
    )
    .bind(user)
    .bind(purpose)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO account_tokens (token_digest, user_id, purpose, created_at, expires_at) \
         VALUES ($1, $2, $3, \
             CASE WHEN $4 THEN now() - interval '2 days' ELSE now() END, \
             CASE WHEN $4 THEN now() - interval '1 day' ELSE now() + interval '15 minutes' END)",
    )
    .bind(token.digest())
    .bind(user)
    .bind(purpose)
    .bind(expired)
    .execute(pool)
    .await
    .unwrap();
    token
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn verification_is_single_use_expiring_and_purpose_bound_under_concurrency() {
    let (pool, accounts, user) = setup().await;
    let expired = fixture_token(&pool, user, "email_verification", true).await;
    assert_eq!(
        accounts.verify_email(expired.expose_secret()).await,
        Err(AccountError::InvalidToken)
    );
    let reset = fixture_token(&pool, user, "password_reset", false).await;
    assert_eq!(
        accounts.verify_email(reset.expose_secret()).await,
        Err(AccountError::InvalidToken)
    );
    assert_eq!(
        accounts.verify_email("not-a-token").await,
        Err(AccountError::InvalidToken)
    );
    let token = fixture_token(&pool, user, "email_verification", false).await;
    let (first, second) = tokio::join!(
        accounts.verify_email(token.expose_secret()),
        accounts.verify_email(token.expose_secret()),
    );
    assert!(matches!(
        (first, second),
        (Ok(()), Err(AccountError::InvalidToken)) | (Err(AccountError::InvalidToken), Ok(()))
    ));
    assert!(
        accounts
            .authenticate("fixture@example.test", PASSWORD)
            .await
            .unwrap()
            .verified
    );
    assert_eq!(
        accounts.verify_email(token.expose_secret()).await,
        Err(AccountError::InvalidToken)
    );
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM delivery_jobs")
        .fetch_one(&pool)
        .await
        .unwrap();
    accounts.request_verification(user).await.unwrap();
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM delivery_jobs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(before, after, "verified accounts do not get resend jobs");
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn resend_and_reset_requests_store_only_intents_with_generic_acknowledgements() {
    let (pool, accounts, user) = setup().await;
    accounts.request_verification(user).await.unwrap();
    accounts
        .request_password_reset(" Fixture@Example.Test ")
        .await
        .unwrap();
    accounts
        .request_password_reset("unknown@example.test")
        .await
        .unwrap();
    let verification: i64 =
        sqlx::query_scalar("SELECT count(*) FROM delivery_jobs WHERE kind = 'verification_email'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let reset: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM delivery_jobs WHERE kind = 'password_reset_email'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((verification, reset), (2, 1));
    assert!(
        accounts
            .authenticate("fixture@example.test", PASSWORD)
            .await
            .is_ok()
    );
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn failed_smtp_preserves_credentials_and_does_not_claim_provider_cleanup() {
    let (pool, accounts, user) = setup().await;
    let values = HashMap::from([
        (
            "DATABASE_URL".into(),
            std::env::var("DATABASE_URL").unwrap(),
        ),
        ("CSRF_SECRET".into(), std::env::var("CSRF_SECRET").unwrap()),
        (
            "RATE_LIMIT_SECRET".into(),
            std::env::var("RATE_LIMIT_SECRET").unwrap(),
        ),
        ("SMTP_HOST".into(), "127.0.0.1".into()),
        ("SMTP_PORT".into(), "1".into()),
    ]);
    let config = Config::from_values(&values).unwrap();
    let delivery = EmailDelivery::new(pool.clone(), &config).unwrap();
    let key_job = jobs::enqueue(
        &mut pool.acquire().await.unwrap(),
        &Intent::RevokeKey {
            key_id: "key_test_fixture".into(),
        },
    )
    .await
    .unwrap();
    let result = jobs::run_batch_for(
        &pool,
        &[Kind::VerificationEmail, Kind::PasswordResetEmail],
        move |claim| {
            let delivery = delivery.clone();
            async move { delivery.deliver(&claim).await.map_err(|_| DeliveryFailure) }
        },
    )
    .await
    .unwrap();
    assert_eq!((result.claimed, result.retried), (1, 1));
    let attempts: i32 = sqlx::query_scalar("SELECT attempts FROM delivery_jobs WHERE id = $1")
        .bind(key_job)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(attempts, 0);
    assert!(
        !accounts
            .authenticate("fixture@example.test", PASSWORD)
            .await
            .unwrap()
            .verified
    );
    let digests: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM account_tokens WHERE user_id = $1 AND octet_length(token_digest) = 32",
    ).bind(user).fetch_one(&pool).await.unwrap();
    assert_eq!(digests, 1);
    assert!(matches!(
        jobs::claim_batch_for(&pool, 1, &[]).await,
        Err(jobs::JobError::InvalidBatch)
    ));
    pool.close().await;
}
