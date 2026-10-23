use super::{Actor, Social, SocialError};
use uuid::Uuid;

impl Social {
    pub async fn set_following(
        &self,
        actor: Actor,
        target: Uuid,
        following: bool,
    ) -> Result<(), SocialError> {
        self.write(actor).await?;
        if actor.id == target {
            return Err(SocialError::SelfFollow);
        }
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| SocialError::Unavailable)?;
        // Keep the target alive until the relationship change commits. A racing
        // account deletion must not turn an absent target into an FK/storage 503.
        let exists: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM users WHERE id = $1 FOR KEY SHARE")
                .bind(target)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|_| SocialError::Unavailable)?;
        if exists.is_none() {
            return Err(SocialError::NotFound);
        }
        if following {
            sqlx::query("INSERT INTO follows (follower_id, followee_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
                .bind(actor.id).bind(target).execute(&mut *tx).await.map_err(|_| SocialError::Unavailable)?;
        } else {
            sqlx::query("DELETE FROM follows WHERE follower_id = $1 AND followee_id = $2")
                .bind(actor.id)
                .bind(target)
                .execute(&mut *tx)
                .await
                .map_err(|_| SocialError::Unavailable)?;
        }
        tx.commit().await.map_err(|_| SocialError::Unavailable)
    }

    pub async fn set_like(&self, actor: Actor, post: Uuid, liked: bool) -> Result<(), SocialError> {
        self.write(actor).await?;
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| SocialError::Unavailable)?;
        let exists: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM posts WHERE id = $1 FOR KEY SHARE")
                .bind(post)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|_| SocialError::Unavailable)?;
        if exists.is_none() {
            return Err(SocialError::NotFound);
        }
        if liked {
            sqlx::query(
                "INSERT INTO likes (user_id, post_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            )
            .bind(actor.id)
            .bind(post)
            .execute(&mut *tx)
            .await
            .map_err(|_| SocialError::Unavailable)?;
        } else {
            sqlx::query("DELETE FROM likes WHERE user_id = $1 AND post_id = $2")
                .bind(actor.id)
                .bind(post)
                .execute(&mut *tx)
                .await
                .map_err(|_| SocialError::Unavailable)?;
        }
        tx.commit().await.map_err(|_| SocialError::Unavailable)
    }
}
