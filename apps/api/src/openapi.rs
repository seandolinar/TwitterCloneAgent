//! Public contract generated from the same DTOs/handlers as both HTTP adapters.
use crate::{gateway, http::ErrorEnvelope, social::*, social_http};
use axum::http::Method;
use serde_json::{Value, json};
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Twitter clone social API",
        version = "1.0.0",
        description = "All /v1 operations require an Unkey Bearer key belonging to a verified account. Permissions are the intersection of gateway and local grants. No account or key management is available through this API. Text lengths count Unicode code points. Application errors and gateway errors have different envelopes; see docs/API.md."
    ),
    paths(
        social_http::me,
        social_http::update_profile,
        social_http::profile,
        social_http::profile_posts,
        social_http::feed,
        social_http::post,
        social_http::create_post,
        social_http::delete_post,
        social_http::follow,
        social_http::unfollow,
        social_http::like,
        social_http::unlike
    ),
    components(schemas(Profile, ProfileUpdate, Post, PostCreate, Page, ErrorEnvelope))
)]
struct Contract;

/// Pure generation: no runtime configuration, database, or credentials required.
pub fn document() -> Value {
    let mut doc = serde_json::to_value(Contract::openapi()).expect("OpenAPI serializes");
    doc["servers"] = json!([{
        "url": "http://localhost:8088",
        "description": "Optional local Frontline gateway, not the browser port. Override for your environment."
    }]);
    doc["components"]["securitySchemes"] = json!({
        "bearerKey": {"type":"http", "scheme":"bearer",
            "description":"Personal Unkey API key, not a root credential. Verified once by the gateway."}
    });
    doc["security"] = json!([{"bearerKey":[]}]);
    doc["x-gateway-errors"] = json!({
        "description": "Frontline may reject a request before Rust. Its provider-owned error body is NOT ErrorEnvelope. Do not require requestId or an application error.code at the edge.",
        "statuses": [401, 403, 429, 500, 502, 503, 504],
        "reference": "https://www.unkey.com/docs/compute/gateway/api-key-auth",
        "retry": "Honor gateway Retry-After when supplied. Never automatically replay a POST after an ambiguous transport or 5xx failure."
    });
    for (path, item) in doc["paths"].as_object_mut().expect("paths") {
        for (verb, operation) in item.as_object_mut().expect("path item") {
            let method = Method::from_bytes(verb.to_uppercase().as_bytes()).expect("HTTP method");
            let permission = gateway::permission(&method, path)
                .expect("every documented operation must have a runtime permission");
            operation["x-required-permission"] =
                serde_json::to_value(permission).expect("permission serializes");
            for (status, description) in [
                ("400", "Malformed path, query, or JSON; request_failed."),
                (
                    "401",
                    "Application: invalid_key (unknown, revoked, expired, mismatched, or unverified owner). Gateway may independently reject before Rust.",
                ),
                (
                    "403",
                    "Application: permission_denied or forbidden (not the resource owner).",
                ),
                ("404", "Application: not_found."),
                ("405", "Application: method_not_allowed."),
                (
                    "408",
                    "Application: request_timeout; do not assume a write was rolled back.",
                ),
                (
                    "413",
                    "Application: request_too_large; maximum JSON body is 16 KiB.",
                ),
                ("415", "Application: request_failed; use application/json."),
                (
                    "422",
                    "Application: invalid_post, invalid_profile, self_follow, invalid_pagination, or request_failed. Unknown fields are rejected.",
                ),
                (
                    "429",
                    "Application: shared per-user social write limit; Retry-After seconds. Gateway per-key limit is separate and has its own envelope.",
                ),
                (
                    "503",
                    "Application: service_unavailable or integration_unavailable. Authorization never falls back to success.",
                ),
            ] {
                operation["responses"][status] = json!({
                    "description": description,
                    "content": {"application/json": {"schema": {"$ref":"#/components/schemas/ErrorEnvelope"}}}
                });
            }
            operation["responses"]["429"]["headers"] = json!({
                "Retry-After": {"description":"Seconds until retry", "schema":{"type":"integer","minimum":1}}
            });
        }
    }
    doc
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    #[tokio::test]
    async fn every_runtime_permission_has_a_documented_live_route() {
        let doc = document();
        let documented_count: usize = doc["paths"]
            .as_object()
            .unwrap()
            .values()
            .map(|item| item.as_object().unwrap().len())
            .sum();
        assert_eq!(documented_count, gateway::OPERATIONS.len());
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://127.0.0.1:1/unavailable")
            .unwrap();
        let social = Social::new(pool.clone(), "isolated-contract-fixture-secret", 30).unwrap();
        let app = crate::http::gateway_router(pool, social, Some("ks_fixture".into()));
        for (method, path, permission) in gateway::OPERATIONS {
            let operation = &doc["paths"][path][method.to_lowercase()];
            assert_eq!(operation["x-required-permission"], json!(permission));
            assert!(operation["responses"]["401"]["content"].is_object());
            let path = path
                .replace("{handle}", "alice")
                .replace("{id}", "00000000-0000-0000-0000-000000000001");
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(*method)
                        .uri(path)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                401,
                "{method} must reach authentication, not 404/405"
            );
        }
    }

    #[test]
    fn published_contract_is_current_and_excludes_browser_credentials() {
        let published: Value =
            serde_json::from_str(include_str!("../../web/public/docs/openapi.json"))
                .expect("valid published JSON");
        assert_eq!(published, document(), "regenerate with the openapi binary");
        assert_eq!(published["paths"].as_object().unwrap().len(), 8);
        let schemas = &published["components"]["schemas"];
        assert_eq!(
            schemas["PostCreate"]["properties"]["body"]["maxLength"],
            280
        );
        assert_eq!(schemas["PostCreate"]["additionalProperties"], false);
        assert!(schemas["Profile"]["properties"].get("email").is_none());
        assert!(schemas.get("ErrorDetails").is_some());
        for path in published["paths"].as_object().unwrap().keys() {
            assert!(path.starts_with("/v1/"));
            assert!(!path.contains("auth") && !path.contains("keys"));
        }
    }
}
