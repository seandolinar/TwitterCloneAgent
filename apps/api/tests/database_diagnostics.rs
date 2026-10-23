use twitter_clone_api::database::{MIGRATOR, check_connection};

mod support;

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn diagnostic_is_read_only_and_separates_authentication_and_database_errors() {
    let pool = support::fresh_pool().await;
    // No migrations: success must require only an authenticated SELECT 1.
    let parent = url::Url::parse(&std::env::var("DATABASE_URL").unwrap()).unwrap();
    let name: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&pool)
        .await
        .unwrap();
    let mut url = parent;
    url.set_path(&name);
    check_connection(url.as_str()).await.unwrap();
    let initialized: bool = sqlx::query_scalar("SELECT to_regclass('public.users') IS NOT NULL")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!initialized);
    let mut invalid_password = url.clone();
    invalid_password
        .set_password(Some("wrong-private-password"))
        .unwrap();
    let error = check_connection(invalid_password.as_str())
        .await
        .unwrap_err();
    assert!(error.contains("authentication failed"));
    assert!(!error.contains("wrong-private-password"));
    let mut missing_database = url.clone();
    missing_database.set_path("twitterclone_test_nonexistent_database");
    assert_eq!(
        check_connection(missing_database.as_str())
            .await
            .unwrap_err(),
        "The requested database does not exist"
    );
    MIGRATOR.run(&pool).await.unwrap();
    check_connection(url.as_str()).await.unwrap();
    assert!(check_connection("not-a-database-url").await.is_err());
    assert!(check_connection("https://example.invalid").await.is_err());
    pool.close().await;
}
