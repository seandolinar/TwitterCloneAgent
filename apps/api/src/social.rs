//! Shared social rules. HTTP adapters supply actors, never caller-selected owners.
use crate::{
    accounts::ActiveSession,
    http::ApiError,
    rate_limit::{RateLimitError, RateLimiter, Rule},
};
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

mod posts;
mod relationships;
mod timelines;
pub use posts::{Post, PostCreate};
pub use timelines::{Page, PageRequest};

/// Construct only after the adapter authenticates its credential. Service
/// authorization still checks current account verification and resource ownership.
#[derive(Clone, Copy, Debug)]
pub struct Actor {
    id: Uuid,
}

impl Actor {
    pub(crate) fn from_gateway(id: Uuid) -> Self {
        Self { id }
    }

    pub fn from_session(session: &ActiveSession) -> Self {
        Self {
            id: session.identity.id,
        }
    }

    pub fn id(self) -> Uuid {
        self.id
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SocialError {
    AuthenticationRequired,
    InvalidProfile,
    InvalidPost,
    SelfFollow,
    InvalidPagination,
    Forbidden,
    VerificationRequired,
    NotFound,
    Unavailable,
    RateLimit(RateLimitError),
}

impl IntoResponse for SocialError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            Self::AuthenticationRequired => (
                StatusCode::UNAUTHORIZED,
                "authentication_required",
                "Log in to continue.",
            ),
            Self::InvalidProfile => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_profile",
                "Display names require 1–50 Unicode code points and biographies at most 160.",
            ),
            Self::InvalidPost => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_post",
                "Posts require 1–280 Unicode code points and at least one non-whitespace character.",
            ),
            Self::SelfFollow => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "self_follow",
                "You cannot follow yourself.",
            ),
            Self::InvalidPagination => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_pagination",
                "Use a returned cursor and a page size between 1 and 100.",
            ),
            Self::Forbidden => (
                StatusCode::FORBIDDEN,
                "forbidden",
                "You do not own this resource.",
            ),
            Self::VerificationRequired => (
                StatusCode::FORBIDDEN,
                "verification_required",
                "Verify your email to continue.",
            ),
            Self::NotFound => (
                StatusCode::NOT_FOUND,
                "not_found",
                "The resource was not found.",
            ),
            Self::Unavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "service_unavailable",
                "The service is temporarily unavailable.",
            ),
            Self::RateLimit(error) => return error.into_response(),
        };
        ApiError {
            status,
            code,
            message,
        }
        .into_response()
    }
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: Uuid,
    pub handle: String,
    pub display_name: String,
    pub biography: String,
    pub joined_at: DateTime<Utc>,
    pub follower_count: i64,
    pub following_count: i64,
    pub post_count: i64,
    pub viewer_follows: bool,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfileUpdate {
    #[schema(min_length = 1, max_length = 50)]
    pub display_name: String,
    #[schema(max_length = 160)]
    pub biography: String,
}

impl ProfileUpdate {
    fn validate(&self) -> Result<(), SocialError> {
        if !(1..=50).contains(&self.display_name.chars().count())
            || self.biography.chars().count() > 160
        {
            return Err(SocialError::InvalidProfile);
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct Social {
    pool: PgPool,
    limiter: RateLimiter,
    write_rule: Rule,
}

const PROFILE_SELECT: &str = "SELECT account.id, account.handle, account.display_name, account.biography, \
         account.created_at AS joined_at, \
         (SELECT count(*) FROM follows WHERE followee_id = account.id) AS follower_count, \
         (SELECT count(*) FROM follows WHERE follower_id = account.id) AS following_count, \
         (SELECT count(*) FROM posts WHERE author_id = account.id) AS post_count, \
         EXISTS(SELECT 1 FROM follows WHERE follower_id = $2 AND followee_id = account.id) AS viewer_follows \
     FROM users AS account";

impl Social {
    pub fn new(
        pool: PgPool,
        rate_secret: &str,
        writes_per_minute: u32,
    ) -> Result<Self, SocialError> {
        Ok(Self {
            limiter: RateLimiter::new(pool.clone(), rate_secret).map_err(SocialError::RateLimit)?,
            pool,
            write_rule: Rule::new(writes_per_minute, 60).map_err(SocialError::RateLimit)?,
        })
    }

    pub async fn profile(
        &self,
        handle: &str,
        viewer: Option<Actor>,
    ) -> Result<Profile, SocialError> {
        let query = format!("{PROFILE_SELECT} WHERE account.handle_normalized = $1");
        sqlx::query_as(sqlx::AssertSqlSafe(query))
            .bind(handle.to_ascii_lowercase())
            .bind(viewer.map(Actor::id))
            .fetch_optional(&self.pool)
            .await
            .map_err(|_| SocialError::Unavailable)?
            .ok_or(SocialError::NotFound)
    }

    pub async fn me(&self, actor: Actor) -> Result<Profile, SocialError> {
        let query = format!("{PROFILE_SELECT} WHERE account.id = $1");
        sqlx::query_as(sqlx::AssertSqlSafe(query))
            .bind(actor.id)
            .bind(Some(actor.id))
            .fetch_optional(&self.pool)
            .await
            .map_err(|_| SocialError::Unavailable)?
            .ok_or(SocialError::NotFound)
    }

    pub async fn update_profile(
        &self,
        actor: Actor,
        owner: Uuid,
        input: ProfileUpdate,
    ) -> Result<Profile, SocialError> {
        if actor.id != owner {
            return Err(SocialError::Forbidden);
        }
        self.write(actor).await?;
        input.validate()?;
        sqlx::query("UPDATE users SET display_name = $1, biography = $2 WHERE id = $3")
            .bind(input.display_name)
            .bind(input.biography)
            .bind(owner)
            .execute(&self.pool)
            .await
            .map_err(|_| SocialError::Unavailable)?;
        self.me(actor).await
    }

    async fn verified(&self, actor: Actor) -> Result<(), SocialError> {
        let verified: Option<bool> =
            sqlx::query_scalar("SELECT verified_at IS NOT NULL FROM users WHERE id = $1")
                .bind(actor.id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|_| SocialError::Unavailable)?;
        match verified {
            Some(true) => Ok(()),
            Some(false) => Err(SocialError::VerificationRequired),
            None => Err(SocialError::Forbidden),
        }
    }

    async fn write(&self, actor: Actor) -> Result<(), SocialError> {
        self.verified(actor).await?;
        self.limiter
            .check("social:write:user", actor.id.as_bytes(), self.write_rule)
            .await
            .map_err(SocialError::RateLimit)
    }
}
