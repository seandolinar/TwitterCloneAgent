use sqlx::PgPool;
use twitter_clone_api::database::connect;
use uuid::Uuid;

/// Each test owns a fresh database in the wrapper's disposable PostgreSQL.
/// Validate the parent before creating anything; never reset a caller's database.
pub async fn fresh_pool() -> PgPool {
    let value = std::env::var("DATABASE_URL").expect("use tests/with_postgres.py");
    let parent = connect(&value).await.unwrap();
    let parent_name: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&parent)
        .await
        .unwrap();
    assert!(
        parent_name.starts_with("twitterclone_test_"),
        "refusing a non-test PostgreSQL"
    );
    let name = format!("twitterclone_test_{}", Uuid::new_v4().simple());
    // PostgreSQL cannot bind a database identifier. This audited statement uses
    // only our fixed prefix and UUID-generated hex, never caller-supplied text.
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
        .execute(&parent)
        .await
        .unwrap();
    parent.close().await;
    let mut url = url::Url::parse(&value).unwrap();
    url.set_path(&name);
    connect(url.as_str()).await.unwrap()
}
