use sqlx::{PgPool, Row};
use twitter_clone_api::database::{MIGRATOR, is_ready};
use uuid::Uuid;

mod support;

async fn user(pool: &PgPool, email: &str, handle: &str) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, email_normalized, handle, password_hash, display_name) \
         VALUES ($1, $2, $3, 'test-fixture-not-a-password-hash', $3)",
    )
    .bind(id)
    .bind(email)
    .bind(handle)
    .execute(pool)
    .await?;
    Ok(id)
}

fn constraint(result: Result<sqlx::postgres::PgQueryResult, sqlx::Error>, code: &str) {
    let error = result.expect_err("constraint must reject input");
    assert_eq!(
        error.as_database_error().and_then(|e| e.code()).as_deref(),
        Some(code)
    );
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn initial_schema_constraints_and_repeatable_migrations() {
    let pool = support::fresh_pool().await;
    let name: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        name.starts_with("twitterclone_test_"),
        "refusing a non-test database"
    );
    assert!(!is_ready(&pool).await, "an empty database cannot be ready");
    MIGRATOR.run(&pool).await.unwrap();
    assert!(is_ready(&pool).await);

    let alice = user(&pool, "alice@example.test", "Alice").await.unwrap();
    let bob = user(&pool, "bob@example.test", "Bob").await.unwrap();
    assert!(user(&pool, "alice@example.test", "Another").await.is_err());
    assert!(user(&pool, "other@example.test", "aLiCe").await.is_err());
    assert!(user(&pool, "Upper@example.test", "Upper").await.is_err());
    assert!(
        user(&pool, "invalid@example.test", "bad-handle")
            .await
            .is_err()
    );

    for body in ["", " \t\n", &"x".repeat(281)] {
        constraint(
            sqlx::query("INSERT INTO posts (id, author_id, body) VALUES ($1, $2, $3)")
                .bind(Uuid::new_v4())
                .bind(alice)
                .bind(body)
                .execute(&pool)
                .await,
            "23514",
        );
    }
    let post = Uuid::new_v4();
    sqlx::query("INSERT INTO posts (id, author_id, body) VALUES ($1, $2, $3)")
        .bind(post)
        .bind(alice)
        .bind("🦀".repeat(280))
        .execute(&pool)
        .await
        .unwrap();
    let chars: i32 = sqlx::query_scalar("SELECT char_length(body) FROM posts WHERE id = $1")
        .bind(post)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(chars, 280);

    constraint(
        sqlx::query("INSERT INTO follows (follower_id, followee_id) VALUES ($1, $1)")
            .bind(alice)
            .execute(&pool)
            .await,
        "23514",
    );
    sqlx::query("INSERT INTO follows (follower_id, followee_id) VALUES ($1, $2)")
        .bind(alice)
        .bind(bob)
        .execute(&pool)
        .await
        .unwrap();
    constraint(
        sqlx::query("INSERT INTO follows (follower_id, followee_id) VALUES ($1, $2)")
            .bind(alice)
            .bind(bob)
            .execute(&pool)
            .await,
        "23505",
    );

    sqlx::query("INSERT INTO likes (user_id, post_id) VALUES ($1, $2)")
        .bind(bob)
        .bind(post)
        .execute(&pool)
        .await
        .unwrap();
    constraint(
        sqlx::query("INSERT INTO likes (user_id, post_id) VALUES ($1, $2)")
            .bind(bob)
            .bind(post)
            .execute(&pool)
            .await,
        "23505",
    );
    constraint(
        sqlx::query("INSERT INTO likes (user_id, post_id) VALUES ($1, $2)")
            .bind(bob)
            .bind(Uuid::new_v4())
            .execute(&pool)
            .await,
        "23503",
    );

    constraint(
        sqlx::query(
            "INSERT INTO account_tokens (token_digest, user_id, purpose, expires_at) \
         VALUES ($1, $2, 'email_verification', now() + interval '1 hour')",
        )
        .bind(vec![0_u8; 31])
        .bind(alice)
        .execute(&pool)
        .await,
        "23514",
    );
    for (digest, purpose) in [
        (vec![1_u8; 32], "email_verification"),
        (vec![2_u8; 32], "password_reset"),
    ] {
        sqlx::query(
            "INSERT INTO account_tokens (token_digest, user_id, purpose, expires_at) \
             VALUES ($1, $2, $3, now() + interval '1 hour')",
        )
        .bind(digest)
        .bind(alice)
        .bind(purpose)
        .execute(&pool)
        .await
        .unwrap();
    }
    constraint(
        sqlx::query(
            "INSERT INTO account_tokens (token_digest, user_id, purpose, expires_at) \
         VALUES ($1, $2, 'email_verification', now() + interval '1 hour')",
        )
        .bind(vec![3_u8; 32])
        .bind(alice)
        .execute(&pool)
        .await,
        "23505",
    );

    constraint(
        sqlx::query("INSERT INTO delivery_jobs (id, kind) VALUES ($1, 'verification_email')")
            .bind(Uuid::new_v4())
            .execute(&pool)
            .await,
        "23514",
    );
    sqlx::query(
        "INSERT INTO delivery_jobs (id, kind, user_id) VALUES ($1, 'verification_email', $2)",
    )
    .bind(Uuid::new_v4())
    .bind(alice)
    .execute(&pool)
    .await
    .unwrap();

    for permissions in [vec!["root.admin"], vec![]] {
        constraint(sqlx::query(
            "INSERT INTO api_keys \
             (key_id, user_id, keyspace_id, label, masked_hint, permissions, expires_at) \
             VALUES ('key_fixture', $1, 'ks_fixture', 'Test', 'masked', $2, now() + interval '90 days')",
        ).bind(alice).bind(permissions).execute(&pool).await, "23514");
    }
    sqlx::query(
        "INSERT INTO api_keys \
         (key_id, user_id, keyspace_id, label, masked_hint, permissions, expires_at) \
         VALUES ('key_fixture', $1, 'ks_fixture', 'Test', 'masked', ARRAY['posts.read'], now() + interval '90 days')",
    ).bind(alice).execute(&pool).await.unwrap();

    // A second migration pass must preserve existing application/security state.
    MIGRATOR.run(&pool).await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 2);
    let columns = sqlx::query(
        "SELECT column_name FROM information_schema.columns \
         WHERE table_schema = 'public' AND table_name IN ('api_keys', 'delivery_jobs')",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    for row in columns {
        let column: &str = row.get("column_name");
        assert!(!["secret", "key", "password", "token", "body", "payload"].contains(&column));
    }
    sqlx::query("DELETE FROM posts WHERE id = $1")
        .bind(post)
        .execute(&pool)
        .await
        .unwrap();
    let likes: i64 = sqlx::query_scalar("SELECT count(*) FROM likes")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(likes, 0);
    sqlx::query("UPDATE _sqlx_migrations SET success = false WHERE version = 1")
        .execute(&pool)
        .await
        .unwrap();
    assert!(!is_ready(&pool).await, "a failed migration cannot be ready");
    sqlx::query("UPDATE _sqlx_migrations SET success = true WHERE version = 1")
        .execute(&pool)
        .await
        .unwrap();
    assert!(is_ready(&pool).await);
    sqlx::query("ALTER TABLE delivery_jobs RENAME TO missing_jobs")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        !is_ready(&pool).await,
        "missing required tables cannot be ready"
    );
    sqlx::query("ALTER TABLE missing_jobs RENAME TO delivery_jobs")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    assert!(
        !is_ready(&pool).await,
        "an unavailable pool cannot be ready"
    );
}
