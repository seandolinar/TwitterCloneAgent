use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
    routing::get,
};
use sqlx::{PgPool, Row};
use tokio::task::JoinSet;
use tower::ServiceExt;
use twitter_clone_api::{
    database::MIGRATOR,
    rate_limit::{RateLimitError, RateLimiter, Rule, cleanup_expired},
};
use uuid::Uuid;

mod support;

async fn test_pool() -> PgPool {
    let pool = support::fresh_pool().await;
    let name: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        name.starts_with("twitterclone_test_"),
        "refusing a non-test database"
    );
    MIGRATOR.run(&pool).await.unwrap();
    pool
}

fn limiter(pool: PgPool) -> RateLimiter {
    let secret = std::env::var("RATE_LIMIT_SECRET").expect("use tests/with_postgres.py");
    RateLimiter::new(pool, &secret).unwrap()
}

fn app(limiter: RateLimiter, action: String, rule: Rule) -> Router {
    Router::new().route(
        "/limited",
        get(move || {
            let limiter = limiter.clone();
            let action = action.clone();
            async move {
                limiter
                    .check(&action, b"alice@example.test|203.0.113.8", rule)
                    .await?;
                Ok::<_, RateLimitError>(StatusCode::NO_CONTENT)
            }
        }),
    )
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn concurrent_http_apps_with_separate_pools_share_one_limit() {
    let first_pool = test_pool().await;
    let second_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(10)
        .connect_with((*first_pool.connect_options()).clone())
        .await
        .unwrap();
    let action = format!("parallel-{}", Uuid::new_v4());
    // A long test window avoids a wall-clock boundary during concurrent requests.
    let rule = Rule::new(7, 1_000_000_000).unwrap();
    let first = app(limiter(first_pool.clone()), action.clone(), rule);
    let second = app(limiter(second_pool.clone()), action.clone(), rule);
    let mut clients = JoinSet::new();
    for index in 0..32 {
        let app = if index % 2 == 0 {
            first.clone()
        } else {
            second.clone()
        };
        clients.spawn(async move {
            app.oneshot(
                Request::builder()
                    .uri("/limited")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
        });
    }
    let mut allowed = 0;
    let mut denied = 0;
    while let Some(response) = clients.join_next().await {
        let response = response.unwrap();
        match response.status() {
            StatusCode::NO_CONTENT => allowed += 1,
            StatusCode::TOO_MANY_REQUESTS => {
                denied += 1;
                let retry: u64 = response.headers()[header::RETRY_AFTER]
                    .to_str()
                    .unwrap()
                    .parse()
                    .unwrap();
                assert!(retry > 0);
            }
            status => panic!("unexpected rate-limit status {status}"),
        }
    }
    assert_eq!((allowed, denied), (7, 25));
    let rows =
        sqlx::query("SELECT hits, identifier_digest FROM rate_limit_buckets WHERE action = $1")
            .bind(&action)
            .fetch_all(&first_pool)
            .await
            .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get::<i64, _>("hits"), 7);
    assert_eq!(rows[0].get::<Vec<u8>, _>("identifier_digest").len(), 32);
    first_pool.close().await;
    second_pool.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn windows_reset_and_cleanup_is_bounded_without_deleting_active_limits() {
    let pool = test_pool().await;
    let limiter = limiter(pool.clone());
    let action = format!("expiry-{}", Uuid::new_v4());
    let rule = Rule::new(1, 1_000_000_000).unwrap();
    limiter
        .check(&action, b"test-identifier", rule)
        .await
        .unwrap();
    assert!(matches!(
        limiter.check(&action, b"test-identifier", rule).await,
        Err(RateLimitError::Limited { .. })
    ));
    sqlx::query(
        "UPDATE rate_limit_buckets SET window_start = now() - interval '2 minutes', \
         expires_at = now() - interval '1 minute' WHERE action = $1",
    )
    .bind(&action)
    .execute(&pool)
    .await
    .unwrap();
    limiter
        .check(&action, b"test-identifier", rule)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO rate_limit_buckets \
         (action, identifier_digest, window_start, hits, expires_at) \
         VALUES ($1, $2, now() - interval '3 minutes', 1, now() - interval '2 minutes')",
    )
    .bind(&action)
    .bind(vec![0_u8; 32])
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(cleanup_expired(&pool, 1).await.unwrap(), 1);
    let remaining: i64 =
        sqlx::query_scalar("SELECT count(*) FROM rate_limit_buckets WHERE action = $1")
            .bind(&action)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(remaining, 2);
    assert_eq!(cleanup_expired(&pool, 1).await.unwrap(), 1);
    assert_eq!(cleanup_expired(&pool, 1).await.unwrap(), 0);
    assert!(matches!(
        limiter.check(&action, b"test-identifier", rule).await,
        Err(RateLimitError::Limited { .. })
    ));
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn unavailable_storage_never_authorizes_a_request() {
    let pool = test_pool().await;
    let limiter = limiter(pool.clone());
    pool.close().await;
    assert_eq!(
        limiter
            .check("outage", b"identifier", Rule::new(5, 60).unwrap())
            .await,
        Err(RateLimitError::Unavailable),
    );
    let response = app(limiter, "outage".into(), Rule::new(5, 60).unwrap())
        .oneshot(
            Request::builder()
                .uri("/limited")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}
