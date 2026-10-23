use axum::{
    Router,
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{Request, StatusCode, header},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{collections::HashMap, net::SocketAddr, sync::Arc};
use tower::ServiceExt;
use twitter_clone_api::{
    accounts::Accounts,
    browser::BrowserAuth,
    config::Config,
    database::MIGRATOR,
    http::{browser_router, gateway_router},
    social::Social,
    tokens::BearerToken,
};
use uuid::Uuid;

mod support;
const PASSWORD: &str = "gateway fixture account password";
const ALL: &[&str] = &[
    "profiles.read",
    "profiles.write",
    "posts.read",
    "posts.write",
    "feed.read",
    "follows.write",
    "likes.write",
];
const READ: &[&str] = &["profiles.read", "posts.read", "feed.read"];

async fn setup(limit: u32) -> (PgPool, Arc<BrowserAuth>, Router, Router) {
    setup_with_origin(limit, "http://localhost:3000").await
}

async fn setup_with_origin(limit: u32, origin: &str) -> (PgPool, Arc<BrowserAuth>, Router, Router) {
    let pool = support::fresh_pool().await;
    MIGRATOR.run(&pool).await.unwrap();
    let mut values: HashMap<_, _> = ["DATABASE_URL", "CSRF_SECRET", "RATE_LIMIT_SECRET"]
        .into_iter()
        .map(|key| (key.into(), std::env::var(key).unwrap()))
        .collect();
    values.insert("APP_ORIGIN".into(), origin.into());
    let auth = Arc::new(
        BrowserAuth::new(
            pool.clone(),
            Arc::new(Config::from_values(&values).unwrap()),
        )
        .await
        .unwrap(),
    );
    let social = Social::new(
        pool.clone(),
        &std::env::var("RATE_LIMIT_SECRET").unwrap(),
        limit,
    )
    .unwrap();
    let browser = browser_router(pool.clone(), &auth, social.clone());
    let gateway = gateway_router(pool.clone(), social, Some("ks_fixture".into()));
    (pool, auth, browser, gateway)
}

async fn user(pool: &PgPool, accounts: &Accounts, handle: &str) -> (Uuid, BearerToken) {
    let email = format!("{handle}@example.test");
    accounts.register(&email, handle, PASSWORD).await.unwrap();
    let grant = accounts.login(&email, PASSWORD, None).await.unwrap();
    sqlx::query("UPDATE users SET verified_at=now() WHERE id=$1")
        .bind(grant.identity.id)
        .execute(pool)
        .await
        .unwrap();
    (grant.identity.id, grant.token)
}

async fn key(pool: &PgPool, user: Uuid, id: &str, permissions: &[&str]) {
    sqlx::query("INSERT INTO api_keys (key_id,user_id,keyspace_id,label,masked_hint,permissions,expires_at) \
        VALUES ($1,$2,'ks_fixture','Fixture','tc_…hint',$3,now()+interval '1 day')")
        .bind(id).bind(user).bind(permissions).execute(pool).await.unwrap();
}

fn principal(user: Uuid, key: &str, permissions: &[&str]) -> Value {
    json!({"version":"v1","type":"API_KEY","subject":user,"identity":{"externalId":user},
        "source":{"key":{"keyId":key,"keySpaceId":"ks_fixture","permissions":permissions}}})
}
fn request(method: &str, path: &str, principal: Option<&Value>, body: Value) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(principal) = principal {
        request = request.header("x-unkey-principal", principal.to_string());
    }
    request.body(Body::from(body.to_string())).unwrap()
}
async fn response(app: &Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&body).unwrap()
        },
    )
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn integration_principals_require_version_type_identity_keyspace_and_registered_owner() {
    let (pool, auth, _, gateway) = setup(30).await;
    let (alice, token) = user(&pool, &auth.accounts, "alice").await;
    let (bob, _) = user(&pool, &auth.accounts, "bob").await;
    key(&pool, alice, "key_fixture", ALL).await;
    let good = principal(alice, "key_fixture", ALL);
    assert_eq!(
        response(&gateway, request("GET", "/v1/me", Some(&good), Value::Null))
            .await
            .0,
        StatusCode::OK
    );
    let mut future = good.clone();
    future["futureField"] = json!(true);
    future["identity"]["meta"] = json!({"ignored":"fixture"});
    assert_eq!(
        response(
            &gateway,
            request("GET", "/v1/me", Some(&future), Value::Null)
        )
        .await
        .0,
        StatusCode::OK
    );
    let mut invalids = Vec::new();
    for (field, value) in [
        ("version", json!("v2")),
        ("type", json!("USER")),
        ("subject", json!(bob)),
    ] {
        let mut invalid = good.clone();
        invalid[field] = value;
        invalids.push(invalid);
    }
    let mut invalid = good.clone();
    invalid.as_object_mut().unwrap().remove("identity");
    invalids.push(invalid);
    let mut invalid = good.clone();
    invalid["identity"]["externalId"] = json!("not-a-user-uuid");
    invalid["subject"] = json!("not-a-user-uuid");
    invalids.push(invalid);
    invalids.push(principal(bob, "key_fixture", ALL));
    let mut invalid = good.clone();
    invalid["source"]["key"]["keySpaceId"] = json!("ks_other");
    invalids.push(invalid);
    invalids.push(principal(alice, "key_unregistered", ALL));
    let mut invalid = good.clone();
    invalid["source"]["key"]["expiresAt"] = json!(0);
    invalids.push(invalid);
    for invalid in invalids {
        let (status, error) = response(
            &gateway,
            request(
                "POST",
                "/v1/posts",
                Some(&invalid),
                json!({"body":"denied"}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(error["error"]["code"], "invalid_key");
        assert!(!error.to_string().contains("externalId"));
    }
    let mut malformed = request("GET", "/v1/me", None, Value::Null);
    malformed.headers_mut().insert(
        "x-unkey-principal",
        "malformed-private-fixture".parse().unwrap(),
    );
    assert_eq!(
        response(&gateway, malformed).await.0,
        StatusCode::UNAUTHORIZED
    );
    let mut duplicate = request("GET", "/v1/me", Some(&good), Value::Null);
    duplicate
        .headers_mut()
        .append("x-unkey-principal", good.to_string().parse().unwrap());
    assert_eq!(
        response(&gateway, duplicate).await.0,
        StatusCode::UNAUTHORIZED
    );
    let mut cookie_only = request("GET", "/v1/me", None, Value::Null);
    cookie_only.headers_mut().insert(
        header::COOKIE,
        format!("twitterclone_dev_session={}", token.expose_secret())
            .parse()
            .unwrap(),
    );
    cookie_only.headers_mut().insert(
        header::AUTHORIZATION,
        "Bearer raw-key-without-a-principal".parse().unwrap(),
    );
    assert_eq!(
        response(&gateway, cookie_only).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        response(
            &gateway,
            request("POST", "/v1/auth/keys", Some(&good), json!({}))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        response(
            &gateway,
            request("POST", "/v1/auth/login", Some(&good), json!({}))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let posts: i64 = sqlx::query_scalar("SELECT count(*) FROM posts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(posts, 0);
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn permissions_are_the_intersection_and_resource_ownership_stays_in_shared_services() {
    let (pool, auth, _, gateway) = setup(30).await;
    let (alice, _) = user(&pool, &auth.accounts, "alice").await;
    let (bob, _) = user(&pool, &auth.accounts, "bob").await;
    key(&pool, alice, "key_read_only", READ).await;
    key(&pool, alice, "key_all", ALL).await;
    for principal in [
        principal(alice, "key_read_only", ALL),
        principal(alice, "key_all", READ),
        principal(alice, "key_all", &[]),
    ] {
        let (status, error) = response(
            &gateway,
            request(
                "POST",
                "/v1/posts",
                Some(&principal),
                json!({"body":"denied"}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(error["error"]["code"], "permission_denied");
    }
    let bob_post = Uuid::new_v4();
    sqlx::query("INSERT INTO posts(id,author_id,body) VALUES($1,$2,'Bob fixture')")
        .bind(bob_post)
        .bind(bob)
        .execute(&pool)
        .await
        .unwrap();
    let good = principal(alice, "key_all", ALL);
    assert_eq!(
        response(
            &gateway,
            request(
                "DELETE",
                &format!("/v1/posts/{bob_post}"),
                Some(&good),
                Value::Null
            )
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        response(
            &gateway,
            request(
                "PATCH",
                "/v1/me",
                Some(&good),
                json!({"displayName":"spoof","biography":"","userId":bob})
            )
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let created = response(
        &gateway,
        request(
            "POST",
            "/v1/posts",
            Some(&good),
            json!({"body":"Alice fixture"}),
        ),
    )
    .await;
    assert_eq!(created.0, StatusCode::CREATED);
    assert_eq!(created.1["authorId"], alice.to_string());
    let feed = response(
        &gateway,
        request("GET", "/v1/feed", Some(&good), Value::Null),
    )
    .await;
    assert_eq!(feed.1["items"].as_array().unwrap().len(), 1);
    assert_eq!(feed.1["items"][0]["authorId"], alice.to_string());
    assert_eq!(
        response(
            &gateway,
            request(
                "PUT",
                &format!("/v1/me/following/{bob}"),
                Some(&good),
                Value::Null
            )
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        response(
            &gateway,
            request(
                "PUT",
                &format!("/v1/posts/{bob_post}/like"),
                Some(&good),
                Value::Null
            )
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let feed = response(
        &gateway,
        request("GET", "/v1/feed", Some(&good), Value::Null),
    )
    .await;
    assert_eq!(feed.1["items"].as_array().unwrap().len(), 2);
    for path in [
        "/v1/profiles/bob".to_owned(),
        "/v1/profiles/bob/posts".to_owned(),
        format!("/v1/posts/{bob_post}"),
    ] {
        assert_eq!(
            response(&gateway, request("GET", &path, Some(&good), Value::Null))
                .await
                .0,
            StatusCode::OK
        );
    }
    assert_eq!(
        response(
            &gateway,
            request(
                "PATCH",
                "/v1/me",
                Some(&good),
                json!({"displayName":"Alice edited","biography":"fixture"})
            )
        )
        .await
        .0,
        StatusCode::OK
    );
    for _ in 0..2 {
        assert_eq!(
            response(
                &gateway,
                request(
                    "DELETE",
                    &format!("/v1/me/following/{bob}"),
                    Some(&good),
                    Value::Null
                )
            )
            .await
            .0,
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            response(
                &gateway,
                request(
                    "DELETE",
                    &format!("/v1/posts/{bob_post}/like"),
                    Some(&good),
                    Value::Null
                )
            )
            .await
            .0,
            StatusCode::NO_CONTENT
        );
    }
    assert_eq!(
        response(
            &gateway,
            request(
                "DELETE",
                &format!("/v1/posts/{}", created.1["id"].as_str().unwrap()),
                Some(&good),
                Value::Null
            )
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn revoked_expired_unverified_and_storage_failure_never_authorize() {
    let (pool, auth, _, gateway) = setup(30).await;
    let (alice, _) = user(&pool, &auth.accounts, "alice").await;
    key(&pool, alice, "key_fixture", ALL).await;
    let good = principal(alice, "key_fixture", ALL);
    sqlx::query("UPDATE api_keys SET revoked_at=now()")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        response(&gateway, request("GET", "/v1/me", Some(&good), Value::Null))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    sqlx::query("UPDATE api_keys SET revoked_at=NULL,created_at=now()-interval '2 days',expires_at=now()-interval '1 day'").execute(&pool).await.unwrap();
    assert_eq!(
        response(&gateway, request("GET", "/v1/me", Some(&good), Value::Null))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    sqlx::query("UPDATE api_keys SET expires_at=now()+interval '1 day'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE api_keys SET keyspace_id='ks_other'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        response(&gateway, request("GET", "/v1/me", Some(&good), Value::Null))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    sqlx::query("UPDATE api_keys SET keyspace_id='ks_fixture'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE users SET verified_at=NULL")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        response(&gateway, request("GET", "/v1/me", Some(&good), Value::Null))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    pool.close().await;
    assert_eq!(
        response(&gateway, request("GET", "/v1/me", Some(&good), Value::Null))
            .await
            .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn password_reset_denies_new_read_and_write_requests_with_a_previously_valid_cached_principal()
 {
    let (pool, auth, _, gateway) = setup(30).await;
    let (alice, token) = user(&pool, &auth.accounts, "alice").await;
    key(&pool, alice, "key_fixture", ALL).await;
    let good = principal(alice, "key_fixture", ALL);
    assert_eq!(
        response(
            &gateway,
            request(
                "POST",
                "/v1/posts",
                Some(&good),
                json!({"body":"before reset"})
            )
        )
        .await
        .0,
        StatusCode::CREATED
    );
    let reset = BearerToken::generate().unwrap();
    sqlx::query("INSERT INTO account_tokens(token_digest,user_id,purpose,expires_at) VALUES($1,$2,'password_reset',now()+interval '30 minutes')")
        .bind(reset.digest()).bind(alice).execute(&pool).await.unwrap();
    auth.accounts
        .reset_password(
            reset.expose_secret(),
            "replacement gateway fixture password",
        )
        .await
        .unwrap();
    assert_eq!(
        response(&gateway, request("GET", "/v1/me", Some(&good), Value::Null))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        response(
            &gateway,
            request(
                "POST",
                "/v1/posts",
                Some(&good),
                json!({"body":"after reset denied"})
            )
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert!(
        auth.accounts
            .resolve_session(token.expose_secret())
            .await
            .unwrap()
            .is_none()
    );
    let posts: i64 = sqlx::query_scalar("SELECT count(*) FROM posts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(posts, 1);
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn browser_and_gateway_share_one_user_write_budget_without_identity_fallback() {
    let (pool, auth, browser, gateway) = setup(3).await;
    let (alice, token) = user(&pool, &auth.accounts, "alice").await;
    key(&pool, alice, "key_fixture", ALL).await;
    let good = principal(alice, "key_fixture", ALL);
    let mut cookies = format!("twitterclone_dev_session={}", token.expose_secret());
    let browser_request = |method: &str, path: &str, cookies: &str, csrf: &str, body: Value| {
        let mut request = request(method, path, None, body);
        request
            .headers_mut()
            .insert(header::COOKIE, cookies.parse().unwrap());
        request
            .headers_mut()
            .insert(header::ORIGIN, "http://localhost:3000".parse().unwrap());
        request
            .headers_mut()
            .insert("x-csrf-token", csrf.parse().unwrap());
        request
            .extensions_mut()
            .insert(ConnectInfo("192.0.2.1:1000".parse::<SocketAddr>().unwrap()));
        request
    };
    let bootstrap = browser
        .clone()
        .oneshot(browser_request(
            "GET",
            "/browser/auth/csrf",
            &cookies,
            "",
            Value::Null,
        ))
        .await
        .unwrap();
    let cookie = cookie::Cookie::parse(
        bootstrap.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .to_owned(),
    )
    .unwrap();
    cookies.push_str(&format!("; {}={}", cookie.name(), cookie.value()));
    let body: Value =
        serde_json::from_slice(&to_bytes(bootstrap.into_body(), 16384).await.unwrap()).unwrap();
    let csrf = body["csrfToken"].as_str().unwrap();
    let created = response(
        &browser,
        browser_request(
            "POST",
            "/browser/v1/posts",
            &cookies,
            csrf,
            json!({"body":"browser write"}),
        ),
    )
    .await;
    assert_eq!(created.0, StatusCode::CREATED);
    assert_eq!(
        response(
            &gateway,
            request(
                "PUT",
                &format!("/v1/posts/{}/like", created.1["id"].as_str().unwrap()),
                Some(&good),
                Value::Null
            )
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        response(
            &browser,
            browser_request(
                "PATCH",
                "/browser/v1/me",
                &cookies,
                csrf,
                json!({"displayName":"Alice","biography":""})
            )
        )
        .await
        .0,
        StatusCode::OK
    );
    let limited = gateway
        .clone()
        .oneshot(request(
            "POST",
            "/v1/posts",
            Some(&good),
            json!({"body":"limited gateway write"}),
        ))
        .await
        .unwrap();
    assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(
        limited.headers()[header::RETRY_AFTER]
            .to_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
            > 0
    );
    let mut forged = browser_request("GET", "/browser/v1/me", "", csrf, Value::Null);
    forged
        .headers_mut()
        .insert("x-unkey-principal", good.to_string().parse().unwrap());
    forged
        .headers_mut()
        .insert(header::AUTHORIZATION, "Bearer fixture".parse().unwrap());
    assert_eq!(response(&browser, forged).await.0, StatusCode::UNAUTHORIZED);
    let posts: i64 = sqlx::query_scalar("SELECT count(*) FROM posts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(posts, 1);
    pool.close().await;
}

// Run the distributed example against real Rust HTTP and PostgreSQL. Only the
// upstream provider/proxy boundaries below are fixtures, not runtime bypasses.
#[allow(dead_code)]
#[path = "../examples/social_api.rs"]
mod social_example;

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn executable_documented_requests_match_the_application_contract() {
    use axum::{
        extract::Request as AxumRequest,
        middleware::{self, Next},
    };
    let web_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let web = format!("http://{}", web_listener.local_addr().unwrap());
    let (pool, auth, browser, gateway) = setup_with_origin(30, &web).await;
    let (alice, token) = user(&pool, &auth.accounts, "example_alice").await;
    key(&pool, alice, "key_example_write", ALL).await;
    key(&pool, alice, "key_example_read", READ).await;
    let gateway = gateway.layer(middleware::from_fn(
        move |mut request: AxumRequest, next: Next| async move {
            // Replay the same upstream principal after revocation: local denial
            // cannot depend on provider propagation.
            request.headers_mut().remove("x-unkey-principal");
            let principal = match request
                .headers()
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
            {
                Some("Bearer example-write-fixture") => {
                    Some(principal(alice, "key_example_write", ALL))
                }
                Some("Bearer example-read-fixture") => {
                    Some(principal(alice, "key_example_read", READ))
                }
                _ => None,
            };
            if let Some(principal) = principal {
                request
                    .headers_mut()
                    .insert("x-unkey-principal", principal.to_string().parse().unwrap());
            }
            next.run(request).await
        },
    ));
    let browser = Router::new()
        .fallback_service(browser)
        .layer(middleware::from_fn(
            |mut request: AxumRequest, next: Next| async move {
                let path = request
                    .uri()
                    .path()
                    .replacen("/api/auth/", "/browser/auth/", 1);
                *request.uri_mut() = path.parse().unwrap();
                next.run(request).await
            },
        ));
    let gateway_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let gateway_url = format!("http://{}", gateway_listener.local_addr().unwrap());
    let web_server = tokio::spawn(async move {
        axum::serve(
            web_listener,
            browser.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let gateway_server = tokio::spawn(async move {
        axum::serve(gateway_listener, gateway).await.unwrap();
    });
    let result = social_example::run(social_example::Inputs {
        gateway: gateway_url,
        browser: web,
        write_key: "example-write-fixture".into(),
        read_key: "example-read-fixture".into(),
        read_key_id: "key_example_read".into(),
        session_cookie: format!("twitterclone_dev_session={}", token.expose_secret()),
    })
    .await;
    web_server.abort();
    gateway_server.abort();
    result.unwrap();
    let posts: i64 = sqlx::query_scalar("SELECT count(*) FROM posts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(posts, 0, "example deletes only its two known created posts");
    let revoked: bool = sqlx::query_scalar(
        "SELECT revoked_at IS NOT NULL FROM api_keys WHERE key_id='key_example_read'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(revoked);
    pool.close().await;
}
