//! Trusted private integration ingress only. The unsigned provider principal is
//! meaningful only behind Frontline/Compute, never on a publicly reachable port.
use crate::{
    http::ApiError,
    social::{Actor, Social},
    social_http::{self, SocialContext},
    unkey::Permission,
};
use axum::{
    Router,
    extract::{MatchedPath, Request, State},
    http::{Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Deserialize)]
struct Principal {
    version: String,
    subject: String,
    #[serde(rename = "type")]
    kind: String,
    identity: Identity,
    source: Source,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Identity {
    external_id: String,
}
#[derive(Deserialize)]
struct Source {
    key: PrincipalKey,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrincipalKey {
    key_id: String,
    key_space_id: String,
    expires_at: Option<i64>,
    #[serde(default)]
    permissions: Vec<String>,
}

struct Gateway {
    pool: PgPool,
    keyspace: Option<String>,
}

pub(crate) fn routes(pool: PgPool, social: Social, keyspace: Option<String>) -> Router {
    Router::new()
        .nest("/v1", social_http::routes(social))
        .layer(middleware::from_fn_with_state(
            Arc::new(Gateway { pool, keyspace }),
            authenticate,
        ))
}

// This is the complete social permission map, shared with contract coverage
// tests. An added route without a rule fails closed instead of reaching a handler.
pub(crate) const OPERATIONS: &[(&str, &str, Permission)] = &[
    ("GET", "/v1/me", Permission::ProfilesRead),
    ("PATCH", "/v1/me", Permission::ProfilesWrite),
    ("GET", "/v1/profiles/{handle}", Permission::ProfilesRead),
    ("GET", "/v1/profiles/{handle}/posts", Permission::PostsRead),
    ("GET", "/v1/posts/{id}", Permission::PostsRead),
    ("GET", "/v1/feed", Permission::FeedRead),
    ("POST", "/v1/posts", Permission::PostsWrite),
    ("DELETE", "/v1/posts/{id}", Permission::PostsWrite),
    ("PUT", "/v1/me/following/{id}", Permission::FollowsWrite),
    ("DELETE", "/v1/me/following/{id}", Permission::FollowsWrite),
    ("PUT", "/v1/posts/{id}/like", Permission::LikesWrite),
    ("DELETE", "/v1/posts/{id}/like", Permission::LikesWrite),
];

pub(crate) fn permission(method: &Method, path: &str) -> Option<Permission> {
    OPERATIONS
        .iter()
        .find(|(verb, route, _)| *verb == method.as_str() && *route == path)
        .map(|(_, _, permission)| *permission)
}

fn denied() -> ApiError {
    ApiError {
        status: StatusCode::UNAUTHORIZED,
        code: "invalid_key",
        message: "A valid integration key is required.",
    }
}

fn principal(request: &Request) -> Result<Principal, ApiError> {
    let mut values = request.headers().get_all("x-unkey-principal").iter();
    let value = values.next().ok_or_else(denied)?;
    if values.next().is_some() || value.as_bytes().len() > 16 * 1024 {
        return Err(denied());
    }
    let principal: Principal = serde_json::from_slice(value.as_bytes()).map_err(|_| denied())?;
    if principal.version != "v1"
        || principal.kind != "API_KEY"
        || principal.subject != principal.identity.external_id
        || Uuid::parse_str(&principal.identity.external_id).is_err()
    {
        return Err(denied());
    }
    Ok(principal)
}

async fn authenticate(
    State(state): State<Arc<Gateway>>,
    mut request: Request,
    next: Next,
) -> Response {
    let Some(path) = request.extensions().get::<MatchedPath>() else {
        return next.run(request).await;
    };
    let Some(required) = permission(request.method(), path.as_str()) else {
        return ApiError {
            status: StatusCode::METHOD_NOT_ALLOWED,
            code: "method_not_allowed",
            message: "The method is not allowed.",
        }
        .into_response();
    };
    let principal = match principal(&request) {
        Ok(value) => value,
        Err(error) => return error.into_response(),
    };
    let Some(expected_keyspace) = &state.keyspace else {
        return ApiError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "integration_unavailable",
            message: "API key integration requires setup.",
        }
        .into_response();
    };
    if &principal.source.key.key_space_id != expected_keyspace {
        return denied().into_response();
    }
    let owner = Uuid::parse_str(&principal.identity.external_id).expect("validated UUID");
    let row: Result<Option<(Uuid, Vec<String>)>, _> = sqlx::query_as(
        "SELECT account.id, api_key.permissions FROM api_keys AS api_key \
         JOIN users AS account ON account.id = api_key.user_id \
         WHERE api_key.key_id = $1 AND api_key.keyspace_id = $2 AND api_key.user_id = $3 \
         AND api_key.revoked_at IS NULL AND api_key.expires_at > clock_timestamp() \
         AND account.verified_at IS NOT NULL \
         AND ($4::bigint IS NULL OR $4 > extract(epoch FROM clock_timestamp()) * 1000)",
    )
    .bind(&principal.source.key.key_id)
    .bind(expected_keyspace)
    .bind(owner)
    .bind(principal.source.key.expires_at)
    .fetch_optional(&state.pool)
    .await;
    let (id, local_permissions) = match row {
        Ok(Some(row)) => row,
        Ok(None) => return denied().into_response(),
        Err(_) => {
            return ApiError {
                status: StatusCode::SERVICE_UNAVAILABLE,
                code: "service_unavailable",
                message: "The service is temporarily unavailable.",
            }
            .into_response();
        }
    };
    // The enum's serialization is the one source for provider permission slugs.
    let required: String =
        serde_json::from_value(serde_json::to_value(required).expect("permission serializes"))
            .expect("permission serializes to string");
    if !local_permissions.contains(&required)
        || !principal.source.key.permissions.contains(&required)
    {
        return ApiError {
            status: StatusCode::FORBIDDEN,
            code: "permission_denied",
            message: "This key does not have the required permission.",
        }
        .into_response();
    }
    // Neither credential is consumed by shared social handlers. Removing them
    // also keeps a raw Bearer key away from future request-level diagnostics.
    request.headers_mut().remove(header::COOKIE);
    request.headers_mut().remove(header::AUTHORIZATION);
    request.extensions_mut().insert(SocialContext {
        actor: Some(Actor::from_gateway(id)),
    });
    next.run(request).await
}
