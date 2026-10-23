use sqlx::{ConnectOptions, PgPool, migrate::Migrator, postgres::PgConnectOptions};
use std::{str::FromStr, time::Duration};

pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

pub async fn connect(url: &str) -> Result<PgPool, sqlx::Error> {
    let options = PgConnectOptions::from_str(url)?.disable_statement_logging();
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(5))
        .connect_with(options)
        .await
}

pub async fn is_ready(pool: &PgPool) -> bool {
    tokio::time::timeout(Duration::from_secs(1), check_schema(pool))
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or(false)
}

async fn check_schema(pool: &PgPool) -> Result<bool, sqlx::Error> {
    let applied: Vec<(i64, bool, Vec<u8>)> =
        sqlx::query_as("SELECT version, success, checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(pool)
            .await?;
    let expected: Vec<_> = MIGRATOR.iter().collect();
    if applied.len() != expected.len()
        || applied
            .iter()
            .zip(expected)
            .any(|((version, success, checksum), migration)| {
                !success
                    || *version != migration.version
                    || checksum.as_slice() != migration.checksum.as_ref()
            })
    {
        return Ok(false);
    }
    sqlx::query_scalar(
        "SELECT bool_and(to_regclass(name) IS NOT NULL) \
         FROM unnest(ARRAY[\
            'public.users', 'public.sessions', 'public.account_tokens', \
            'public.posts', 'public.follows', 'public.likes', \
            'public.api_keys', 'public.rate_limit_buckets', 'public.delivery_jobs'\
         ]) AS required(name)",
    )
    .fetch_one(pool)
    .await
}
