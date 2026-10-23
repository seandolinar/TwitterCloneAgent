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

async fn me(
    State(social): State<Arc<Social>>,
    Extension(ctx): Extension<SocialContext>,
) -> Result<Json<Profile>, SocialError> {
    social.me(ctx.required()?).await.map(Json)
}

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

async fn profile(
    State(social): State<Arc<Social>>,
    Extension(ctx): Extension<SocialContext>,
    Path(handle): Path<String>,
) -> Result<Json<Profile>, SocialError> {
    social.profile(&handle, ctx.actor).await.map(Json)
}

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

async fn feed(
    State(social): State<Arc<Social>>,
    Extension(ctx): Extension<SocialContext>,
    Query(page): Query<PageRequest>,
) -> Result<Json<Page>, SocialError> {
    social.feed(ctx.required()?, page).await.map(Json)
}

async fn post(
    State(social): State<Arc<Social>>,
    Extension(ctx): Extension<SocialContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<Post>, SocialError> {
    social.post(id, ctx.actor).await.map(Json)
}

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
