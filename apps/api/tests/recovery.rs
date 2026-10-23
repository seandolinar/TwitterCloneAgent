use sqlx::PgPool;
use twitter_clone_api::{
    accounts::{AccountError, Accounts},
    database::MIGRATOR,
    tokens::BearerToken,
};
use uuid::Uuid;

mod support;
const OLD: &str = "disposable old recovery password";
const NEW: &str = "  disposable new recovery password  ";

async fn setup() -> (PgPool, Accounts, Uuid) {
    let pool = support::fresh_pool().await;
    MIGRATOR.run(&pool).await.unwrap();
    let accounts = Accounts::new(pool.clone()).await.unwrap();
    accounts
        .register("fixture@example.test", "fixture", OLD)
        .await
        .unwrap();
    let owner = accounts
        .authenticate("fixture@example.test", OLD)
        .await
        .unwrap()
        .id;
    sqlx::query("UPDATE users SET verified_at = now() WHERE id = $1")
        .bind(owner)
        .execute(&pool)
        .await
        .unwrap();
    (pool, accounts, owner)
}

async fn token(pool: &PgPool, owner: Uuid, purpose: &str, expired: bool) -> BearerToken {
    let token = BearerToken::generate().unwrap();
    sqlx::query(
        "INSERT INTO account_tokens (token_digest, user_id, purpose, created_at, expires_at) \
         VALUES ($1, $2, $3, now() - interval '1 hour', \
            CASE WHEN $4 THEN now() - interval '1 second' ELSE now() + interval '30 minutes' END)",
    )
    .bind(token.digest())
    .bind(owner)
    .bind(purpose)
    .bind(expired)
    .execute(pool)
    .await
    .unwrap();
    token
}

async fn key(pool: &PgPool, owner: Uuid, id: &str) {
    sqlx::query(
        "INSERT INTO api_keys (key_id, user_id, keyspace_id, label, masked_hint, permissions, expires_at) \
         VALUES ($1, $2, 'ks_fixture', 'fixture', 'masked', ARRAY['posts.read'], now() + interval '1 day')",
    ).bind(id).bind(owner).execute(pool).await.unwrap();
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn reset_atomically_replaces_password_revokes_sessions_and_queues_all_owner_keys() {
    let (pool, accounts, owner) = setup().await;
    accounts
        .register("other@example.test", "other", OLD)
        .await
        .unwrap();
    let other = accounts
        .login("other@example.test", OLD, None)
        .await
        .unwrap();
    let first = accounts
        .login("fixture@example.test", OLD, None)
        .await
        .unwrap();
    let second = accounts
        .login("fixture@example.test", OLD, None)
        .await
        .unwrap();
    key(&pool, owner, "key_fixture_one").await;
    key(&pool, owner, "key_fixture_two").await;
    key(&pool, other.identity.id, "key_other").await;
    let reset = token(&pool, owner, "password_reset", false).await;
    let verify = token(&pool, owner, "email_verification", false).await;
    accounts
        .reset_password(reset.expose_secret(), NEW)
        .await
        .unwrap();
    assert!(matches!(
        accounts.authenticate("fixture@example.test", OLD).await,
        Err(AccountError::InvalidCredentials)
    ));
    assert!(
        accounts
            .authenticate("fixture@example.test", NEW)
            .await
            .is_ok()
    );
    assert!(matches!(
        accounts
            .authenticate("fixture@example.test", NEW.trim())
            .await,
        Err(AccountError::InvalidCredentials)
    ));
    for session in [first, second] {
        assert!(
            accounts
                .resolve_session(session.token.expose_secret())
                .await
                .unwrap()
                .is_none()
        );
    }
    assert!(
        accounts
            .resolve_session(other.token.expose_secret())
            .await
            .unwrap()
            .is_some()
    );
    let states: Vec<(String, bool, bool)> = sqlx::query_as(
        "SELECT key_id, revoked_at IS NOT NULL, provider_revoked_at IS NOT NULL FROM api_keys ORDER BY key_id",
    ).fetch_all(&pool).await.unwrap();
    assert_eq!(
        states,
        vec![
            ("key_fixture_one".into(), true, false),
            ("key_fixture_two".into(), true, false),
            ("key_other".into(), false, false)
        ]
    );
    // The local admission predicate must deny newly presented owner keys even
    // though provider cleanup is still pending. Live gateway denial is task 6.4.
    let active: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM api_keys WHERE user_id = $1 AND revoked_at IS NULL AND expires_at > clock_timestamp()",
    ).bind(owner).fetch_one(&pool).await.unwrap();
    assert_eq!(active, 0);
    let jobs: Vec<String> = sqlx::query_scalar(
        "SELECT key_id FROM delivery_jobs WHERE kind = 'unkey_revoke' ORDER BY key_id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(jobs, vec!["key_fixture_one", "key_fixture_two"]);
    let verify_active: bool = sqlx::query_scalar(
        "SELECT consumed_at IS NULL FROM account_tokens WHERE token_digest = $1",
    )
    .bind(verify.digest())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        verify_active,
        "reset never consumes a verification-purpose token"
    );
    assert_eq!(
        accounts.reset_password(reset.expose_secret(), NEW).await,
        Err(AccountError::InvalidToken)
    );
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn invalid_expired_and_wrong_purpose_reset_do_not_change_credentials() {
    let (pool, accounts, owner) = setup().await;
    let session = accounts
        .login("fixture@example.test", OLD, None)
        .await
        .unwrap();
    let expired = token(&pool, owner, "password_reset", true).await;
    let verification = token(&pool, owner, "email_verification", false).await;
    for value in [
        "not-a-token",
        expired.expose_secret(),
        verification.expose_secret(),
    ] {
        assert_eq!(
            accounts.reset_password(value, NEW).await,
            Err(AccountError::InvalidToken)
        );
    }
    sqlx::query("UPDATE account_tokens SET consumed_at = now() WHERE purpose = 'password_reset'")
        .execute(&pool)
        .await
        .unwrap();
    let valid = token(&pool, owner, "password_reset", false).await;
    for invalid in ["short", &"x".repeat(129)] {
        assert_eq!(
            accounts
                .reset_password(valid.expose_secret(), invalid)
                .await,
            Err(AccountError::InvalidPassword)
        );
    }
    assert!(
        accounts
            .authenticate("fixture@example.test", OLD)
            .await
            .is_ok()
    );
    assert!(
        accounts
            .resolve_session(session.token.expose_secret())
            .await
            .unwrap()
            .is_some()
    );
    accounts
        .reset_password(valid.expose_secret(), NEW)
        .await
        .unwrap();
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn concurrent_resets_have_one_winner_and_old_password_login_cannot_survive_commit() {
    let (pool, accounts, owner) = setup().await;
    let reset = token(&pool, owner, "password_reset", false).await;
    key(&pool, owner, "key_fixture").await;
    let second_accounts = Accounts::new(pool.clone()).await.unwrap();
    let (a, b, login) = tokio::join!(
        accounts.reset_password(reset.expose_secret(), NEW),
        second_accounts.reset_password(reset.expose_secret(), "different disposable new password"),
        accounts.login("fixture@example.test", OLD, None),
    );
    assert!(matches!(
        (a, b),
        (Ok(()), Err(AccountError::InvalidToken)) | (Err(AccountError::InvalidToken), Ok(()))
    ));
    match login {
        Ok(grant) => assert!(
            accounts
                .resolve_session(grant.token.expose_secret())
                .await
                .unwrap()
                .is_none()
        ),
        Err(error) => assert_eq!(error, AccountError::InvalidCredentials),
    }
    let jobs: i64 =
        sqlx::query_scalar("SELECT count(*) FROM delivery_jobs WHERE kind = 'unkey_revoke'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        jobs, 1,
        "racing reset cannot enqueue duplicate provider cleanup"
    );
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn failed_cleanup_enqueue_rolls_back_every_reset_change() {
    let (pool, accounts, owner) = setup().await;
    let session = accounts
        .login("fixture@example.test", OLD, None)
        .await
        .unwrap();
    key(&pool, owner, "key_fixture").await;
    let reset = token(&pool, owner, "password_reset", false).await;
    // Test-owned schema fault: prove late transactional failure cannot leave
    // the new password, consumed token, or revoked sessions/keys committed.
    sqlx::query("ALTER TABLE delivery_jobs ADD CONSTRAINT fixture_no_cleanup CHECK (kind <> 'unkey_revoke')")
        .execute(&pool).await.unwrap();
    assert_eq!(
        accounts.reset_password(reset.expose_secret(), NEW).await,
        Err(AccountError::Unavailable)
    );
    assert!(
        accounts
            .authenticate("fixture@example.test", OLD)
            .await
            .is_ok()
    );
    assert!(
        accounts
            .resolve_session(session.token.expose_secret())
            .await
            .unwrap()
            .is_some()
    );
    let active: bool =
        sqlx::query_scalar("SELECT revoked_at IS NULL FROM api_keys WHERE key_id = 'key_fixture'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(active);
    let unused: bool = sqlx::query_scalar(
        "SELECT consumed_at IS NULL FROM account_tokens WHERE token_digest = $1",
    )
    .bind(reset.digest())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(unused);
    sqlx::query("ALTER TABLE delivery_jobs DROP CONSTRAINT fixture_no_cleanup")
        .execute(&pool)
        .await
        .unwrap();
    accounts
        .reset_password(reset.expose_secret(), NEW)
        .await
        .unwrap();
    pool.close().await;
}
