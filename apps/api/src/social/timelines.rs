use super::{Actor, Post, Social, SocialError, posts::POST_SELECT};
use chrono::{DateTime, Utc};
use data_encoding::BASE64URL_NOPAD;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageRequest {
    pub limit: Option<String>,
    pub cursor: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub items: Vec<Post>,
    pub next_cursor: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    v: u8,
    created_at: DateTime<Utc>,
    id: Uuid,
}

impl PageRequest {
    fn parse(&self) -> Result<(u32, Option<Cursor>), SocialError> {
        let limit = match self.limit.as_deref() {
            None => 20,
            Some(value) if !value.is_empty() && value.bytes().all(|c| c.is_ascii_digit()) => value
                .parse::<u32>()
                .ok()
                .filter(|n| (1..=100).contains(n))
                .ok_or(SocialError::InvalidPagination)?,
            Some(_) => return Err(SocialError::InvalidPagination),
        };
        let cursor = match &self.cursor {
            None => None,
            Some(value) => {
                if value.len() > 512 {
                    return Err(SocialError::InvalidPagination);
                }
                let bytes = BASE64URL_NOPAD
                    .decode(value.as_bytes())
                    .map_err(|_| SocialError::InvalidPagination)?;
                let cursor: Cursor =
                    serde_json::from_slice(&bytes).map_err(|_| SocialError::InvalidPagination)?;
                if cursor.v != 1 {
                    return Err(SocialError::InvalidPagination);
                }
                Some(cursor)
            }
        };
        Ok((limit, cursor))
    }
}

impl Social {
    pub async fn profile_posts(
        &self,
        handle: &str,
        viewer: Option<Actor>,
        page: PageRequest,
    ) -> Result<Page, SocialError> {
        let (limit, cursor) = page.parse()?;
        let owner = self.profile(handle, viewer).await?.id;
        self.timeline(owner, viewer, limit, cursor, false).await
    }

    pub async fn feed(&self, actor: Actor, page: PageRequest) -> Result<Page, SocialError> {
        self.verified(actor).await?;
        let (limit, cursor) = page.parse()?;
        self.timeline(actor.id, Some(actor), limit, cursor, true)
            .await
    }

    async fn timeline(
        &self,
        owner: Uuid,
        viewer: Option<Actor>,
        limit: u32,
        cursor: Option<Cursor>,
        feed: bool,
    ) -> Result<Page, SocialError> {
        // Only fixed server-owned SQL fragments are composed here. Cursor,
        // owner, viewer, and page size always remain bound parameters.
        let authors = if feed {
            "(post.author_id = $1 OR EXISTS(SELECT 1 FROM follows \
                WHERE follower_id = $1 AND followee_id = post.author_id))"
        } else {
            "post.author_id = $1"
        };
        let query = format!(
            "{POST_SELECT} WHERE {authors} \
            AND ($3::timestamptz IS NULL OR (post.created_at, post.id) < ($3, $4::uuid)) \
            ORDER BY post.created_at DESC, post.id DESC LIMIT $5"
        );
        let mut items: Vec<Post> = sqlx::query_as(sqlx::AssertSqlSafe(query))
            .bind(owner)
            .bind(viewer.map(Actor::id))
            .bind(cursor.as_ref().map(|c| c.created_at))
            .bind(cursor.as_ref().map(|c| c.id))
            .bind(i64::from(limit) + 1)
            .fetch_all(&self.pool)
            .await
            .map_err(|_| SocialError::Unavailable)?;
        let has_more = items.len() > limit as usize;
        items.truncate(limit as usize);
        let next_cursor = if has_more {
            let last = items
                .last()
                .expect("positive limit with lookahead has a last item");
            let bytes = serde_json::to_vec(&Cursor {
                v: 1,
                created_at: last.created_at,
                id: last.id,
            })
            .map_err(|_| SocialError::Unavailable)?;
            Some(BASE64URL_NOPAD.encode(&bytes))
        } else {
            None
        };
        Ok(Page { items, next_cursor })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pagination_bounds_and_strict_versioned_cursor_validation() {
        assert_eq!(PageRequest::default().parse().unwrap().0, 20);
        for limit in ["1", "20", "100"] {
            assert!(
                PageRequest {
                    limit: Some(limit.into()),
                    cursor: None
                }
                .parse()
                .is_ok()
            );
        }
        for limit in ["0", "101", "-1", "+1", " 20", "1.5", "", "4294967296"] {
            assert!(
                PageRequest {
                    limit: Some(limit.into()),
                    cursor: None
                }
                .parse()
                .is_err()
            );
        }
        let valid =
            serde_json::json!({"v":1, "created_at":"2026-01-01T00:00:00Z", "id":Uuid::new_v4()});
        let encoded = BASE64URL_NOPAD.encode(valid.to_string().as_bytes());
        assert!(
            PageRequest {
                limit: None,
                cursor: Some(encoded.clone())
            }
            .parse()
            .is_ok()
        );
        for value in [
            "".into(),
            "not base64?".into(),
            format!("{encoded}="),
            "a".repeat(513),
        ] {
            assert!(
                PageRequest {
                    limit: None,
                    cursor: Some(value)
                }
                .parse()
                .is_err()
            );
        }
        for invalid in [
            serde_json::json!({"v":2,"created_at":"2026-01-01T00:00:00Z","id":Uuid::new_v4()}),
            serde_json::json!({"v":1,"created_at":"invalid","id":Uuid::new_v4()}),
            serde_json::json!({"v":1,"created_at":"2026-01-01T00:00:00Z","id":"invalid"}),
            serde_json::json!({"v":1,"created_at":"2026-01-01T00:00:00Z","id":Uuid::new_v4(),"owner":"forged"}),
        ] {
            assert!(
                PageRequest {
                    limit: None,
                    cursor: Some(BASE64URL_NOPAD.encode(invalid.to_string().as_bytes()))
                }
                .parse()
                .is_err()
            );
        }
    }
}
