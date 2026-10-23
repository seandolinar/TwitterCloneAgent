//! Transport-neutral social handlers. Each ingress supplies its own authenticated
//! request context; handlers and services never inspect cookies or gateway headers.
use crate::social::{
    Actor, Page, PageRequest, Post, PostCreate, Profile, ProfileUpdate, Social, SocialError,
};
use axum::{
    Extension, Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, put},
};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone, Copy)]
pub(crate) struct SocialContext {
    pub actor: Option<Actor>,
}

impl SocialContext {
    fn required(self) -> Result<Actor, SocialError> {
        self.actor.ok_or(SocialError::AuthenticationRequired)
    }
}

pub(crate) fn routes(social: Social) -> Router {
    Router::new()
        .route("/me", get(me).patch(update_profile))
        .route("/profiles/{handle}", get(profile))
        .route("/profiles/{handle}/posts", get(profile_posts))
        .route("/feed", get(feed))
        .route("/posts", axum::routing::post(create_post))
        .route("/posts/{id}", get(post).delete(delete_post))
        .route("/me/following/{id}", put(follow).delete(unfollow))
        .route("/posts/{id}/like", put(like).delete(unlike))
        .with_state(Arc::new(social))
}

#[utoipa::path(get, path = "/v1/me", responses((status = 200, body = Profile)))]
async fn me(
    State(social): State<Arc<Social>>,
    Extension(ctx): Extension<SocialContext>,
) -> Result<Json<Profile>, SocialError> {
    social.me(ctx.required()?).await.map(Json)
}

#[utoipa::path(patch, path = "/v1/me", request_body = ProfileUpdate,
    responses((status = 200, body = Profile)))]
async fn update_profile(
    State(social): State<Arc<Social>>,
    Extension(ctx): Extension<SocialContext>,
    Json(input): Json<ProfileUpdate>,
) -> Result<Json<Profile>, SocialError> {
    let actor = ctx.required()?;
    social
        .update_profile(actor, actor.id(), input)
        .await
        .map(Json)
}

#[utoipa::path(get, path = "/v1/profiles/{handle}",
    params(("handle" = String, Path, description = "Case-insensitive immutable handle")),
    responses((status = 200, body = Profile)))]
async fn profile(
    State(social): State<Arc<Social>>,
    Extension(ctx): Extension<SocialContext>,
    Path(handle): Path<String>,
) -> Result<Json<Profile>, SocialError> {
    social.profile(&handle, ctx.actor).await.map(Json)
}

#[utoipa::path(get, path = "/v1/profiles/{handle}/posts",
    params(("handle" = String, Path),
        ("limit" = Option<u32>, Query, minimum = 1, maximum = 100, description = "Default 20; maximum 100"),
        ("cursor" = Option<String>, Query, max_length = 512, description = "Use nextCursor unchanged; newest first by creation time and ID")),
    responses((status = 200, body = Page)))]
async fn profile_posts(
    State(social): State<Arc<Social>>,
    Extension(ctx): Extension<SocialContext>,
    Path(handle): Path<String>,
    Query(page): Query<PageRequest>,
) -> Result<Json<Page>, SocialError> {
    social
        .profile_posts(&handle, ctx.actor, page)
        .await
        .map(Json)
}

#[utoipa::path(get, path = "/v1/feed",
    params(("limit" = Option<u32>, Query, minimum = 1, maximum = 100, description = "Default 20; maximum 100"),
        ("cursor" = Option<String>, Query, max_length = 512, description = "Own plus currently followed authors; opaque nextCursor, newest first")),
    responses((status = 200, body = Page)))]
async fn feed(
    State(social): State<Arc<Social>>,
    Extension(ctx): Extension<SocialContext>,
    Query(page): Query<PageRequest>,
) -> Result<Json<Page>, SocialError> {
    social.feed(ctx.required()?, page).await.map(Json)
}

#[utoipa::path(get, path = "/v1/posts/{id}", params(("id" = Uuid, Path)),
    responses((status = 200, body = Post)))]
async fn post(
    State(social): State<Arc<Social>>,
    Extension(ctx): Extension<SocialContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<Post>, SocialError> {
    social.post(id, ctx.actor).await.map(Json)
}

#[utoipa::path(post, path = "/v1/posts", request_body = PostCreate,
    responses((status = 201, body = Post)))]
async fn create_post(
    State(social): State<Arc<Social>>,
    Extension(ctx): Extension<SocialContext>,
    Json(input): Json<PostCreate>,
) -> Result<(StatusCode, Json<Post>), SocialError> {
    social
        .create_post(ctx.required()?, input)
        .await
        .map(|post| (StatusCode::CREATED, Json(post)))
}

#[utoipa::path(delete, path = "/v1/posts/{id}", params(("id" = Uuid, Path)),
    description = "Author only. Deletes the post and its likes.",
    responses((status = 204, description = "Deleted; empty body")))]
async fn delete_post(
    State(social): State<Arc<Social>>,
    Extension(ctx): Extension<SocialContext>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, SocialError> {
    social
        .delete_post(ctx.required()?, id)
        .await
        .map(|()| StatusCode::NO_CONTENT)
}

#[utoipa::path(put, path = "/v1/me/following/{id}", params(("id" = Uuid, Path)),
    description = "Idempotent. The target must exist and cannot be yourself.",
    responses((status = 204, description = "Following; empty body")))]
async fn follow(
    State(social): State<Arc<Social>>,
    Extension(ctx): Extension<SocialContext>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, SocialError> {
    social
        .set_following(ctx.required()?, id, true)
        .await
        .map(|()| StatusCode::NO_CONTENT)
}
#[utoipa::path(delete, path = "/v1/me/following/{id}", params(("id" = Uuid, Path)),
    description = "Idempotent. The target must exist and cannot be yourself.",
    responses((status = 204, description = "Not following; empty body")))]
async fn unfollow(
    State(social): State<Arc<Social>>,
    Extension(ctx): Extension<SocialContext>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, SocialError> {
    social
        .set_following(ctx.required()?, id, false)
        .await
        .map(|()| StatusCode::NO_CONTENT)
}
#[utoipa::path(put, path = "/v1/posts/{id}/like", params(("id" = Uuid, Path)),
    description = "Idempotent. The post must exist.",
    responses((status = 204, description = "Liked; empty body")))]
async fn like(
    State(social): State<Arc<Social>>,
    Extension(ctx): Extension<SocialContext>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, SocialError> {
    social
        .set_like(ctx.required()?, id, true)
        .await
        .map(|()| StatusCode::NO_CONTENT)
}
#[utoipa::path(delete, path = "/v1/posts/{id}/like", params(("id" = Uuid, Path)),
    description = "Idempotent. The post must exist.",
    responses((status = 204, description = "Not liked; empty body")))]
async fn unlike(
    State(social): State<Arc<Social>>,
    Extension(ctx): Extension<SocialContext>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, SocialError> {
    social
        .set_like(ctx.required()?, id, false)
        .await
        .map(|()| StatusCode::NO_CONTENT)
}
