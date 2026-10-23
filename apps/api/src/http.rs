use axum::{
    BoxError, Json, Router,
    error_handling::HandleErrorLayer,
    extract::{Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Serialize;
use sqlx::PgPool;
use std::time::Duration;
use tower::ServiceBuilder;
use tower_http::limit::RequestBodyLimitLayer;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct RequestId(pub Uuid);

#[derive(Clone, Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: &'static str,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut response = self.status.into_response();
        response.extensions_mut().insert(self);
        response
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorEnvelope {
    error: ErrorDetails,
    request_id: String,
}

#[derive(Serialize)]
struct ErrorDetails {
    code: &'static str,
    message: &'static str,
}

#[derive(Clone, Copy)]
struct HttpLimits {
    body_bytes: usize,
    concurrent_requests: usize,
    request_timeout: Duration,
}

impl Default for HttpLimits {
    fn default() -> Self {
        Self {
            body_bytes: 16 * 1024,
            concurrent_requests: 64,
            request_timeout: Duration::from_secs(10),
        }
    }
}

pub fn browser_router(pool: PgPool) -> Router {
    bounded(
        Router::new()
            .route("/healthz", get(|| async { StatusCode::OK }))
            .route("/readyz", get(readiness))
            .fallback(not_found)
            .method_not_allowed_fallback(method_not_allowed)
            .layer(middleware::from_fn(strip_gateway_identity))
            .with_state(pool),
        HttpLimits::default(),
    )
}

async fn readiness(State(pool): State<PgPool>) -> Result<StatusCode, ApiError> {
    if crate::database::is_ready(&pool).await {
        Ok(StatusCode::OK)
    } else {
        Err(ApiError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "not_ready",
            message: "The database or required schema is unavailable.",
        })
    }
}

pub fn gateway_router() -> Router {
    // No protected route exists until its authorization adapter is implemented.
    bounded(
        Router::new()
            .fallback(not_found)
            .method_not_allowed_fallback(method_not_allowed),
        HttpLimits::default(),
    )
}

fn bounded(router: Router, limits: HttpLimits) -> Router {
    router
        .layer(
            ServiceBuilder::new()
                .layer(HandleErrorLayer::new(layer_error))
                .load_shed()
                .layer(tower::limit::GlobalConcurrencyLimitLayer::new(
                    limits.concurrent_requests,
                ))
                .timeout(limits.request_timeout)
                .layer(RequestBodyLimitLayer::new(limits.body_bytes)),
        )
        .layer(middleware::from_fn(request_context))
}

async fn layer_error(error: BoxError) -> ApiError {
    if error.is::<tower::timeout::error::Elapsed>() {
        ApiError {
            status: StatusCode::REQUEST_TIMEOUT,
            code: "request_timeout",
            message: "The request took too long.",
        }
    } else {
        ApiError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "service_unavailable",
            message: "The service is temporarily unavailable.",
        }
    }
}

async fn not_found() -> ApiError {
    ApiError {
        status: StatusCode::NOT_FOUND,
        code: "not_found",
        message: "The resource was not found.",
    }
}

async fn method_not_allowed() -> ApiError {
    ApiError {
        status: StatusCode::METHOD_NOT_ALLOWED,
        code: "method_not_allowed",
        message: "The method is not allowed.",
    }
}

async fn strip_gateway_identity(mut request: Request, next: Next) -> Response {
    let names: Vec<_> = request
        .headers()
        .keys()
        .filter(|name| name.as_str().starts_with("x-unkey-"))
        .cloned()
        .collect();
    for name in names {
        request.headers_mut().remove(name);
    }
    next.run(request).await
}

async fn request_context(mut request: Request, next: Next) -> Response {
    // Ignore client-supplied IDs, including values containing private information.
    let request_id = Uuid::new_v4();
    request.extensions_mut().insert(RequestId(request_id));
    let mut response = next.run(request).await;
    if response.status().is_client_error() || response.status().is_server_error() {
        let error = response
            .extensions_mut()
            .remove::<ApiError>()
            .unwrap_or_else(|| {
                if response.status() == StatusCode::PAYLOAD_TOO_LARGE {
                    ApiError {
                        status: StatusCode::PAYLOAD_TOO_LARGE,
                        code: "request_too_large",
                        message: "The request body exceeds the allowed size.",
                    }
                } else {
                    ApiError {
                        status: response.status(),
                        code: "request_failed",
                        message: "The request could not be completed.",
                    }
                }
            });
        let headers = response.headers().clone();
        response = (
            error.status,
            Json(ErrorEnvelope {
                error: ErrorDetails {
                    code: error.code,
                    message: error.message,
                },
                request_id: request_id.to_string(),
            }),
        )
            .into_response();
        // Keep protocol headers such as Allow/Retry-After without replacing JSON's type.
        for (name, value) in &headers {
            if name != header::CONTENT_TYPE && name != header::CONTENT_LENGTH {
                response.headers_mut().append(name.clone(), value.clone());
            }
        }
    }
    response.headers_mut().insert(
        "x-request-id",
        HeaderValue::from_str(&request_id.to_string()).expect("UUID is a valid header"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::Request as HttpRequest,
        routing::post,
    };
    use std::sync::Arc;
    use tokio::sync::Notify;
    use tower::ServiceExt;

    fn test_app() -> Router {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://127.0.0.1:1/unavailable")
            .unwrap();
        browser_router(pool)
    }

    fn request(path: &str) -> HttpRequest<Body> {
        HttpRequest::builder()
            .uri(path)
            .body(Body::empty())
            .unwrap()
    }

    #[tokio::test]
    async fn health_is_minimal_and_ids_are_generated_by_server() {
        let mut input = request("/healthz");
        input
            .headers_mut()
            .insert("x-request-id", HeaderValue::from_static("caller-secret"));
        input.headers_mut().insert(
            "x-unkey-principal",
            HeaderValue::from_static("forged-secret"),
        );
        let response = test_app().oneshot(input).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(Uuid::parse_str(response.headers()["x-request-id"].to_str().unwrap()).is_ok());
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert!(
            to_bytes(response.into_body(), 1024)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn errors_are_json_with_request_ids_and_no_request_secrets() {
        let response = test_app()
            .oneshot(request("/missing?token=private"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let request_id = response.headers()["x-request-id"]
            .to_str()
            .unwrap()
            .to_owned();
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["requestId"], request_id);
        assert_eq!(json["error"]["code"], "not_found");
        assert!(!String::from_utf8_lossy(&body).contains("private"));
    }

    #[tokio::test]
    async fn gateway_does_not_expose_browser_health_route() {
        let response = gateway_router().oneshot(request("/healthz")).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn method_errors_use_safe_envelope() {
        let input = HttpRequest::builder()
            .method("POST")
            .uri("/healthz")
            .body(Body::empty())
            .unwrap();
        let response = test_app().oneshot(input).await.unwrap();
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["code"], "method_not_allowed");
    }

    #[tokio::test]
    async fn unavailable_database_is_not_ready() {
        let response = test_app().oneshot(request("/readyz")).await.unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["code"], "not_ready");
        assert!(!String::from_utf8_lossy(&body).contains("postgres://"));
    }

    #[tokio::test]
    async fn request_size_is_bounded() {
        let app = bounded(
            Router::new().route(
                "/body",
                post(|_: axum::body::Bytes| async { StatusCode::NO_CONTENT }),
            ),
            HttpLimits {
                body_bytes: 8,
                ..HttpLimits::default()
            },
        );
        let input = HttpRequest::builder()
            .method("POST")
            .uri("/body")
            .header(header::CONTENT_LENGTH, "9")
            .body(Body::from("123456789"))
            .unwrap();
        let response = app.oneshot(input).await.unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("request_too_large"));
    }

    #[tokio::test]
    async fn slow_requests_time_out_safely() {
        let app = bounded(
            Router::new().route(
                "/slow",
                get(|| async {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    StatusCode::OK
                }),
            ),
            HttpLimits {
                request_timeout: Duration::from_millis(1),
                ..HttpLimits::default()
            },
        );
        let response = app.oneshot(request("/slow")).await.unwrap();
        assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
    }

    #[tokio::test]
    async fn concurrency_limit_sheds_excess_requests() {
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let app = bounded(
            Router::new().route(
                "/held",
                get({
                    let entered = entered.clone();
                    let release = release.clone();
                    move || {
                        let entered = entered.clone();
                        let release = release.clone();
                        async move {
                            entered.notify_one();
                            release.notified().await;
                            StatusCode::OK
                        }
                    }
                }),
            ),
            HttpLimits {
                concurrent_requests: 1,
                ..HttpLimits::default()
            },
        );
        let first = tokio::spawn(app.clone().oneshot(request("/held")));
        entered.notified().await;
        let second = app.oneshot(request("/another-route")).await.unwrap();
        assert_eq!(second.status(), StatusCode::SERVICE_UNAVAILABLE);
        release.notify_one();
        assert_eq!(first.await.unwrap().unwrap().status(), StatusCode::OK);
    }
}
