use sqlx::{ConnectOptions, PgPool, migrate::Migrator, postgres::PgConnectOptions};
use std::{str::FromStr, time::Duration};

pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// Deliberately static diagnostics: driver/server errors can include credentials,
/// identifiers or connection URLs and must not be forwarded to operator logs.
pub fn safe_connection_error(error: &sqlx::Error) -> &'static str {
    match error {
        sqlx::Error::Configuration(_) => "Database URL or connection configuration is invalid",
        sqlx::Error::Tls(_) => {
            "Database TLS/certificate verification failed; check the trusted CA and hostname"
        }
        sqlx::Error::Database(error) => match error.code().as_deref() {
            Some("28P01" | "28000") => {
                "Database authentication failed; check the database username and URL-encoded password"
            }
            Some("3D000") => "The requested database does not exist",
            Some("53300") => "Database connection limit reached",
            Some("57P03") => "Database is not ready to accept connections",
            _ => "Database server rejected the connection; check provider logs",
        },
        sqlx::Error::PoolTimedOut => {
            "Database connection timed out; check network access, provider availability and connection limits"
        }
        sqlx::Error::Io(error) => match error.kind() {
            std::io::ErrorKind::ConnectionRefused => {
                "Database TCP connection was refused; check hostname and port"
            }
            std::io::ErrorKind::TimedOut => "Database network connection timed out",
            std::io::ErrorKind::NotFound => {
                "Database I/O configuration is unavailable; check certificate file paths"
            }
            _ => "Database network/I/O failure; check DNS, IP connectivity and certificate paths",
        },
        _ => "Database connection or query failed; check provider logs",
    }
}

/// Probe the configured endpoint and execute SELECT 1 without migrations or
/// application configuration. A direct connection preserves the actual driver
/// error that pool retry exhaustion can otherwise replace with PoolTimedOut.
pub async fn check_connection(url: &str) -> Result<(), &'static str> {
    use sqlx::Connection;
    let parsed = url::Url::parse(url).map_err(|_| "Database URL is invalid")?;
    if !matches!(parsed.scheme(), "postgres" | "postgresql") {
        return Err("Database URL must use postgres:// or postgresql://");
    }
    let host = parsed
        .host_str()
        .ok_or("Database URL requires a hostname")?;
    let port = parsed.port().unwrap_or(5432);
    let options = PgConnectOptions::from_str(url)
        .map_err(|error| safe_connection_error(&error))?
        .disable_statement_logging();
    let mut addresses = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::lookup_host((host, port)),
    )
    .await
    .map_err(|_| "Database DNS lookup timed out")?
    .map_err(|_| "Database DNS lookup failed")?;
    if addresses.next().is_none() {
        return Err("Database DNS lookup returned no addresses");
    }
    let socket = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::TcpStream::connect((host, port)),
    )
    .await
    .map_err(|_| "Database TCP connection timed out; check the endpoint and network access")?
    .map_err(
        |_| "Database TCP connection failed; check IPv4/IPv6 connectivity, hostname and port",
    )?;
    drop(socket);
    let mut connection = tokio::time::timeout(
        Duration::from_secs(10),
        sqlx::PgConnection::connect_with(&options),
    )
    .await
    .map_err(|_| "Database TLS/authentication handshake timed out")?
    .map_err(|error| safe_connection_error(&error))?;
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        sqlx::query("SELECT 1").execute(&mut connection),
    )
    .await
    .map_err(|_| "Database SELECT 1 timed out")?;
    result.map_err(|error| safe_connection_error(&error))?;
    let _ = tokio::time::timeout(Duration::from_secs(1), connection.close()).await;
    Ok(())
}

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

#[cfg(test)]
mod diagnostic_tests {
    use super::*;

    #[test]
    fn connection_diagnostics_never_include_driver_error_text() {
        let private = "postgres://private-user:private-password@private-host/private-db";
        for error in [
            sqlx::Error::Configuration(private.into()),
            sqlx::Error::Tls(private.into()),
            sqlx::Error::Io(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                private,
            )),
            sqlx::Error::PoolTimedOut,
        ] {
            let message = safe_connection_error(&error);
            assert!(!message.contains(private));
            assert!(!message.contains("private-password"));
        }
    }
}
