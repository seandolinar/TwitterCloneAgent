use cookie::{Cookie, SameSite};
use sqlx::PgPool;
use std::collections::HashMap;
use twitter_clone_api::{
    accounts::{AccountError, Accounts, clear_session_cookie, session_cookie_name},
    config::Config,
    database::MIGRATOR,
    tokens::BearerToken,
};

mod support;

const PASSWORD: &str = "isolated session fixture password";

async fn setup() -> (PgPool, Accounts) {
    let pool = support::fresh_pool().await;
    MIGRATOR.run(&pool).await.unwrap();
    let accounts = Accounts::new(pool.clone()).await.unwrap();
    accounts
        .register("fixture@example.test", "fixture", PASSWORD)
        .await
        .unwrap();
    (pool, accounts)
}

fn config(production: bool) -> Config {
    let mut values = HashMap::from([
        (
            "DATABASE_URL".into(),
            std::env::var("DATABASE_URL").unwrap(),
        ),
        ("CSRF_SECRET".into(), std::env::var("CSRF_SECRET").unwrap()),
        (
            "RATE_LIMIT_SECRET".into(),
            std::env::var("RATE_LIMIT_SECRET").unwrap(),
        ),
    ]);
    if production {
        for (key, value) in [
            ("APP_ENV", "production"),
            ("APP_ORIGIN", "https://social.example.com"),
            ("EMAIL_FROM", "noreply@example.com"),
            ("SMTP_HOST", "smtp.example.com"),
            ("SMTP_PORT", "587"),
            ("SMTP_TLS", "true"),
            ("SMTP_USERNAME", "test-fixture"),
            ("SMTP_PASSWORD", "test-fixture-not-a-real-credential"),
            // Cookie-policy construction only; this URL is never connected.
            (
                "DATABASE_URL",
                "postgres://fixture:fixture@database.example.com/social?sslmode=verify-full",
            ),
        ] {
            values.insert(key.into(), value.into());
        }
    }
    Config::from_values(&values).unwrap()
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn login_rotates_tokens_logout_revokes_and_cookie_policy_is_explicit() {
    let (pool, accounts) = setup().await;
    let first = accounts
        .login("fixture@example.test", PASSWORD, None)
        .await
        .unwrap();
    assert!(
        accounts
            .resolve_session(first.token.expose_secret())
            .await
            .unwrap()
            .is_some()
    );
    assert!(matches!(
        accounts
            .login(
                "fixture@example.test",
                "wrong long fixture password",
                Some(first.token.expose_secret())
            )
            .await,
        Err(AccountError::InvalidCredentials),
    ));
    assert!(
        accounts
            .resolve_session(first.token.expose_secret())
            .await
            .unwrap()
            .is_some()
    );
    let second = accounts
        .login(
            "fixture@example.test",
            PASSWORD,
            Some(first.token.expose_secret()),
        )
        .await
        .unwrap();
    assert!(first.token.expose_secret() != second.token.expose_secret());
    assert!(
        accounts
            .resolve_session(first.token.expose_secret())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        accounts
            .resolve_session(second.token.expose_secret())
            .await
            .unwrap()
            .is_some()
    );
    for production in [false, true] {
        let config = config(production);
        let header = second.cookie(&config);
        let cookie = Cookie::parse(header).unwrap();
        assert_eq!(cookie.name(), session_cookie_name(&config));
        assert!(cookie.value() == second.token.expose_secret());
        assert_eq!(cookie.http_only(), Some(true));
        assert_eq!(cookie.secure().unwrap_or(false), production);
        assert_eq!(cookie.same_site(), Some(SameSite::Lax));
        assert_eq!(cookie.path(), Some("/"));
        assert!(cookie.domain().is_none());
        assert_eq!(cookie.max_age().unwrap().whole_seconds(), 7 * 24 * 60 * 60);
        let removal = Cookie::parse(clear_session_cookie(&config)).unwrap();
        assert_eq!(removal.name(), cookie.name());
        assert_eq!(removal.path(), Some("/"));
        assert_eq!(removal.max_age().unwrap().whole_seconds(), 0);
    }
    let raw = second.token.expose_secret();
    accounts.logout(raw).await.unwrap();
    accounts.logout(raw).await.unwrap();
    assert!(accounts.resolve_session(raw).await.unwrap().is_none());
    assert!(
        accounts
            .resolve_session("not-a-token")
            .await
            .unwrap()
            .is_none()
    );
    let unknown = BearerToken::generate().unwrap();
    assert!(
        accounts
            .resolve_session(unknown.expose_secret())
            .await
            .unwrap()
            .is_none()
    );
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn idle_and_absolute_expiration_are_server_enforced() {
    let (pool, accounts) = setup().await;
    let idle = accounts
        .login("fixture@example.test", PASSWORD, None)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE sessions SET created_at = now() - interval '2 days', \
         last_used_at = now() - interval '25 hours' WHERE token_digest = $1",
    )
    .bind(idle.token.digest())
    .execute(&pool)
    .await
    .unwrap();
    assert!(
        accounts
            .resolve_session(idle.token.expose_secret())
            .await
            .unwrap()
            .is_none()
    );
    let absolute = accounts
        .login("fixture@example.test", PASSWORD, None)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE sessions SET created_at = now() - interval '8 days', \
         expires_at = now() - interval '1 day' WHERE token_digest = $1",
    )
    .bind(absolute.token.digest())
    .execute(&pool)
    .await
    .unwrap();
    assert!(
        accounts
            .resolve_session(absolute.token.expose_secret())
            .await
            .unwrap()
            .is_none()
    );
    let active = accounts
        .login("fixture@example.test", PASSWORD, None)
        .await
        .unwrap();
    let maintenance = tokio::spawn(twitter_clone_api::maintenance::run(pool.clone()));
    let cleanup = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let count: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions")
                .fetch_one(&pool)
                .await
                .unwrap();
            if count == 1 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await;
    maintenance.abort();
    let _ = maintenance.await;
    cleanup.expect("maintenance must remove only expired sessions");
    assert!(
        accounts
            .resolve_session(active.token.expose_secret())
            .await
            .unwrap()
            .is_some()
    );
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn idle_renewal_preserves_absolute_expiry_and_storage_outage_fails_closed() {
    let (pool, accounts) = setup().await;
    let grant = accounts
        .login("fixture@example.test", PASSWORD, None)
        .await
        .unwrap();
    let before: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT expires_at FROM sessions WHERE token_digest = $1")
            .bind(grant.token.digest())
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query(
        "UPDATE sessions SET created_at = now() - interval '2 hours', \
         last_used_at = now() - interval '1 hour' WHERE token_digest = $1",
    )
    .bind(grant.token.digest())
    .execute(&pool)
    .await
    .unwrap();
    assert!(
        accounts
            .resolve_session(grant.token.expose_secret())
            .await
            .unwrap()
            .is_some()
    );
    let (after, renewed): (chrono::DateTime<chrono::Utc>, bool) = sqlx::query_as(
        "SELECT expires_at, last_used_at > now() - interval '5 seconds' \
         FROM sessions WHERE token_digest = $1",
    )
    .bind(grant.token.digest())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(before, after);
    assert!(renewed);
    pool.close().await;
    assert!(matches!(
        accounts.resolve_session(grant.token.expose_secret()).await,
        Err(AccountError::Unavailable),
    ));
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn reauthentication_requires_live_session_and_current_password() {
    let (pool, accounts) = setup().await;
    let grant = accounts
        .login("fixture@example.test", PASSWORD, None)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE sessions SET password_authenticated_at = now() - interval '6 minutes' WHERE token_digest = $1",
    ).bind(grant.token.digest()).execute(&pool).await.unwrap();
    let session = accounts
        .resolve_session(grant.token.expose_secret())
        .await
        .unwrap()
        .unwrap();
    assert!(!session.recent_password_auth);
    assert_eq!(
        accounts
            .reauthenticate(&session, "wrong long fixture password")
            .await,
        Err(AccountError::InvalidCredentials)
    );
    accounts.reauthenticate(&session, PASSWORD).await.unwrap();
    let current = accounts
        .resolve_session(grant.token.expose_secret())
        .await
        .unwrap()
        .unwrap();
    assert!(current.recent_password_auth);
    assert!(current.binding() == grant.token.digest());
    accounts.logout(grant.token.expose_secret()).await.unwrap();
    assert_eq!(
        accounts.reauthenticate(&current, PASSWORD).await,
        Err(AccountError::InvalidCredentials)
    );
    pool.close().await;
}
