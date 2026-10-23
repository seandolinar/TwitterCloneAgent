use super::{Actor, Social, SocialError};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Post {
    pub id: Uuid,
    pub author_id: Uuid,
    pub author_handle: String,
    pub author_display_name: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub like_count: i64,
    pub viewer_likes: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PostCreate {
    pub body: String,
}

impl PostCreate {
    fn validate(&self) -> Result<(), SocialError> {
        if !(1..=280).contains(&self.body.chars().count())
            || self.body.chars().all(char::is_whitespace)
        {
            return Err(SocialError::InvalidPost);
        }
        Ok(())
    }
}

pub(super) const POST_SELECT: &str = "SELECT post.id, post.author_id, \
    author.handle AS author_handle, author.display_name AS author_display_name, \
    post.body, post.created_at, \
    (SELECT count(*) FROM likes WHERE post_id = post.id) AS like_count, \
    EXISTS(SELECT 1 FROM likes WHERE post_id = post.id AND user_id = $2) AS viewer_likes \
    FROM posts AS post JOIN users AS author ON author.id = post.author_id";

impl Social {
    pub async fn post(&self, id: Uuid, viewer: Option<Actor>) -> Result<Post, SocialError> {
        let query = format!("{POST_SELECT} WHERE post.id = $1");
        sqlx::query_as(sqlx::AssertSqlSafe(query))
            .bind(id)
            .bind(viewer.map(Actor::id))
            .fetch_optional(&self.pool)
            .await
            .map_err(|_| SocialError::Unavailable)?
            .ok_or(SocialError::NotFound)
    }

    pub async fn create_post(&self, actor: Actor, input: PostCreate) -> Result<Post, SocialError> {
        self.write(actor).await?;
        input.validate()?;
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO posts (id, author_id, body) VALUES ($1, $2, $3)")
            .bind(id)
            .bind(actor.id)
            .bind(input.body)
            .execute(&self.pool)
            .await
            .map_err(|_| SocialError::Unavailable)?;
        self.post(id, Some(actor)).await
    }

    pub async fn delete_post(&self, actor: Actor, id: Uuid) -> Result<(), SocialError> {
        self.write(actor).await?;
        let owner: Option<Uuid> = sqlx::query_scalar("SELECT author_id FROM posts WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|_| SocialError::Unavailable)?;
        match owner {
            None => return Err(SocialError::NotFound),
            Some(owner) if owner != actor.id => return Err(SocialError::Forbidden),
            Some(_) => {}
        }
        let result = sqlx::query("DELETE FROM posts WHERE id = $1 AND author_id = $2")
            .bind(id)
            .bind(actor.id)
            .execute(&self.pool)
            .await
            .map_err(|_| SocialError::Unavailable)?;
        if result.rows_affected() == 0 {
            return Err(SocialError::NotFound);
        }
        Ok(())
    }
}
