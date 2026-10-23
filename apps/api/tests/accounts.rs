use axum::{http::StatusCode, response::IntoResponse};
use sqlx::Row;
use twitter_clone_api::{
    accounts::{AccountError, Accounts},
    database::MIGRATOR,
};

mod support;

const PASSWORD: &str = "  isolated test fixture password  ";

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn registration_and_credential_checks_preserve_uniqueness_privacy_and_passwords() {
    let pool = support::fresh_pool().await;
    MIGRATOR.run(&pool).await.unwrap();
    let accounts = Accounts::new(pool.clone()).await.unwrap();
    accounts
        .register(" Alice@Example.Test ", "Alice", PASSWORD)
        .await
        .unwrap();
    let identity = accounts
        .authenticate("ALICE@example.test", PASSWORD)
        .await
        .unwrap();
    assert_eq!(identity.handle, "Alice");
    assert!(!identity.verified);
    let row =
        sqlx::query("SELECT email_normalized, password_hash, verified_at FROM users WHERE id = $1")
            .bind(identity.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        row.get::<String, _>("email_normalized"),
        "alice@example.test"
    );
    let hash: String = row.get("password_hash");
    assert!(hash.starts_with("$argon2id$"));
    assert!(!hash.contains(PASSWORD));

    // Repeating the email must not change its password, handle, or job count.
    accounts
        .register(
            "alice@example.test",
            "Alice",
            "different test fixture password",
        )
        .await
        .unwrap();
    accounts
        .register(
            "alice@example.test",
            "Another",
            "different test fixture password",
        )
        .await
        .unwrap();
    assert!(
        accounts
            .authenticate("alice@example.test", PASSWORD)
            .await
            .is_ok()
    );
    assert_eq!(
        accounts
            .authenticate("alice@example.test", "different test fixture password")
            .await,
        Err(AccountError::InvalidCredentials),
    );
    assert_eq!(
        accounts
            .register("bob@example.test", "aLiCe", PASSWORD)
            .await,
        Err(AccountError::HandleTaken),
    );
    let users: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM delivery_jobs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!((users, jobs), (1, 1));

    let missing = accounts
        .authenticate("unknown@example.test", PASSWORD)
        .await
        .unwrap_err();
    let wrong = accounts
        .authenticate("alice@example.test", "wrong but long fixture password")
        .await
        .unwrap_err();
    assert_eq!(missing, wrong);
    assert_eq!(missing.to_string(), wrong.to_string());
    assert_eq!(missing.into_response().status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        accounts
            .authenticate("alice@example.test", PASSWORD.trim())
            .await,
        Err(AccountError::InvalidCredentials)
    );
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn invalid_registration_or_enqueue_failure_cannot_create_partial_accounts() {
    let pool = support::fresh_pool().await;
    MIGRATOR.run(&pool).await.unwrap();
    let accounts = Accounts::new(pool.clone()).await.unwrap();
    for (email, handle, password) in [
        ("invalid email", "valid", PASSWORD),
        ("valid@example.test", "bad-handle", PASSWORD),
        ("valid@example.test", "valid", "short"),
    ] {
        assert!(accounts.register(email, handle, password).await.is_err());
    }
    sqlx::query("ALTER TABLE delivery_jobs RENAME TO temporarily_missing_jobs")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        accounts
            .register("valid@example.test", "valid", PASSWORD)
            .await,
        Err(AccountError::Unavailable),
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    pool.close().await;
}
