use crate::http::ApiError;
use axum::{
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use sqlx::PgPool;
use std::{error::Error, fmt, sync::Arc, time::Duration};

#[derive(Clone, Copy, Debug)]
pub struct Rule {
    limit: u32,
    window_seconds: u32,
}

impl Rule {
    pub const fn new(limit: u32, window_seconds: u32) -> Result<Self, RateLimitError> {
        if limit == 0 || window_seconds == 0 {
            return Err(RateLimitError::InvalidConfiguration);
        }
        Ok(Self {
            limit,
            window_seconds,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateLimitError {
    InvalidConfiguration,
    Unavailable,
    Limited { retry_after: u64 },
}

impl fmt::Display for RateLimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidConfiguration => "Rate-limit configuration is invalid",
            Self::Unavailable => "Rate-limit storage is unavailable",
            Self::Limited { .. } => "Too many requests",
        })
    }
}

impl Error for RateLimitError {}

impl IntoResponse for RateLimitError {
    fn into_response(self) -> Response {
        let error = match self {
            Self::Limited { .. } => ApiError {
                status: StatusCode::TOO_MANY_REQUESTS,
                code: "rate_limited",
                message: "Too many requests. Try again later.",
            },
            Self::InvalidConfiguration | Self::Unavailable => ApiError {
                status: StatusCode::SERVICE_UNAVAILABLE,
                code: "service_unavailable",
                message: "The service is temporarily unavailable.",
            },
        };
        let mut response = error.into_response();
        if let Self::Limited { retry_after } = self {
            response.headers_mut().insert(
                header::RETRY_AFTER,
                HeaderValue::from_str(&retry_after.max(1).to_string())
                    .expect("an integer is a valid header"),
            );
        }
        response
    }
}

// No Debug implementation: the key must not enter diagnostics.
#[derive(Clone)]
pub struct RateLimiter {
    pool: PgPool,
    key: Arc<[u8]>,
}

impl RateLimiter {
    pub fn new(pool: PgPool, secret: &str) -> Result<Self, RateLimitError> {
        if !crate::config::secret_is_valid(secret) {
            return Err(RateLimitError::InvalidConfiguration);
        }
        Ok(Self {
            pool,
            key: Arc::from(secret.as_bytes()),
        })
    }

    /// Consume one attempt in an atomic, database-clock-aligned fixed window.
    ///
    /// Actions/rules are server-owned; identifiers must already be normalized by
    /// the caller. There is no process-local counter or outage fallback.
    pub async fn check(
        &self,
        action: &str,
        identifier: &[u8],
        rule: Rule,
    ) -> Result<(), RateLimitError> {
        if action.is_empty() || action.len() > 128 || identifier.is_empty() {
            return Err(RateLimitError::InvalidConfiguration);
        }
        let digest = self.identifier_digest(action, identifier);
        let operation = sqlx::query_as::<_, (bool, i64)>(
            "WITH tick AS (SELECT clock_timestamp() AS instant), \
             bounds AS ( \
                 SELECT to_timestamp( \
                     floor(extract(epoch FROM instant) / $3::bigint) * $3::bigint \
                 ) AS start FROM tick \
             ), consumed AS ( \
                 INSERT INTO rate_limit_buckets AS bucket \
                     (action, identifier_digest, window_start, hits, expires_at) \
                 SELECT $1, $2, start, 1, start + $3::bigint * interval '1 second' FROM bounds \
                 ON CONFLICT (action, identifier_digest, window_start) \
                 DO UPDATE SET hits = bucket.hits + 1 WHERE bucket.hits < $4::bigint \
                 RETURNING hits \
             ) \
             SELECT EXISTS(SELECT 1 FROM consumed), \
                 greatest(1, ceil(extract(epoch FROM \
                     (bounds.start + $3::bigint * interval '1 second' - clock_timestamp()) \
                 )))::bigint \
             FROM bounds",
        )
        .bind(action)
        .bind(digest)
        .bind(i64::from(rule.window_seconds))
        .bind(i64::from(rule.limit))
        .fetch_one(&self.pool);
        let (allowed, retry_after) = tokio::time::timeout(Duration::from_secs(2), operation)
            .await
            .map_err(|_| RateLimitError::Unavailable)?
            .map_err(|_| RateLimitError::Unavailable)?;
        if allowed {
            Ok(())
        } else {
            Err(RateLimitError::Limited {
                retry_after: retry_after.max(1) as u64,
            })
        }
    }

    fn identifier_digest(&self, action: &str, identifier: &[u8]) -> Vec<u8> {
        let mut mac =
            Hmac::<Sha256>::new_from_slice(&self.key).expect("HMAC accepts a key of any length");
        mac.update(b"twitterclone:rate-limit:v1\0");
        mac.update(&(action.len() as u64).to_be_bytes());
        mac.update(action.as_bytes());
        mac.update(&(identifier.len() as u64).to_be_bytes());
        mac.update(identifier);
        mac.finalize().into_bytes().to_vec()
    }
}

pub async fn cleanup_expired(pool: &PgPool, batch_size: u32) -> Result<u64, RateLimitError> {
    if batch_size == 0 || batch_size > 1000 {
        return Err(RateLimitError::InvalidConfiguration);
    }
    let operation = sqlx::query(
        "WITH expired AS ( \
             SELECT action, identifier_digest, window_start FROM rate_limit_buckets \
             WHERE expires_at <= clock_timestamp() ORDER BY expires_at \
             LIMIT $1 FOR UPDATE SKIP LOCKED \
         ) \
         DELETE FROM rate_limit_buckets AS bucket USING expired \
         WHERE bucket.action = expired.action \
             AND bucket.identifier_digest = expired.identifier_digest \
             AND bucket.window_start = expired.window_start",
    )
    .bind(i64::from(batch_size))
    .execute(pool);
    tokio::time::timeout(Duration::from_secs(2), operation)
        .await
        .map_err(|_| RateLimitError::Unavailable)?
        .map(|result| result.rows_affected())
        .map_err(|_| RateLimitError::Unavailable)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_KEY: &str = "test-fixture-rate-key-not-a-real-application-secret";

    #[tokio::test]
    async fn digests_are_stable_and_separate_actions_and_identifiers() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://127.0.0.1:1/test")
            .unwrap();
        let first = RateLimiter::new(pool.clone(), TEST_KEY).unwrap();
        let second = RateLimiter::new(pool.clone(), TEST_KEY).unwrap();
        let a = first.identifier_digest("login", b"alice@example.test");
        assert_eq!(a.len(), 32);
        assert_eq!(a, second.identifier_digest("login", b"alice@example.test"));
        assert_ne!(a, first.identifier_digest("reset", b"alice@example.test"));
        assert_ne!(a, first.identifier_digest("login", b"bob@example.test"));
        assert_ne!(a, b"alice@example.test");
        let other_key =
            RateLimiter::new(pool, "another-test-fixture-key-not-a-real-secret").unwrap();
        assert_ne!(
            a,
            other_key.identifier_digest("login", b"alice@example.test")
        );
    }

    #[tokio::test]
    async fn invalid_configuration_cannot_disable_limits() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://127.0.0.1:1/test")
            .unwrap();
        for secret in ["", "short", "                                "] {
            assert!(matches!(
                RateLimiter::new(pool.clone(), secret),
                Err(RateLimitError::InvalidConfiguration)
            ));
        }
        assert!(Rule::new(0, 60).is_err());
        assert!(Rule::new(1, 0).is_err());
        let limiter = RateLimiter::new(pool.clone(), TEST_KEY).unwrap();
        let rule = Rule::new(1, 60).unwrap();
        assert_eq!(
            limiter.check("", b"id", rule).await,
            Err(RateLimitError::InvalidConfiguration)
        );
        assert_eq!(
            limiter.check("login", b"", rule).await,
            Err(RateLimitError::InvalidConfiguration)
        );
        assert_eq!(
            cleanup_expired(&pool, 0).await,
            Err(RateLimitError::InvalidConfiguration)
        );
        assert_eq!(
            cleanup_expired(&pool, 1001).await,
            Err(RateLimitError::InvalidConfiguration)
        );
    }

    #[test]
    fn limited_responses_include_positive_retry_after_and_storage_fails_closed() {
        let response = RateLimitError::Limited { retry_after: 0 }.into_response();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.headers()[header::RETRY_AFTER], "1");
        for error in [
            RateLimitError::Unavailable,
            RateLimitError::InvalidConfiguration,
        ] {
            assert_eq!(
                error.into_response().status(),
                StatusCode::SERVICE_UNAVAILABLE
            );
        }
    }
}
