use axum::{
    Router,
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{HeaderMap, Request, StatusCode, header},
};
use cookie::Cookie;
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{collections::HashMap, net::SocketAddr, sync::Arc};
use tower::ServiceExt;
use twitter_clone_api::{
    browser::BrowserAuth, config::Config, database::MIGRATOR, http::browser_router, social::Social,
};
use uuid::Uuid;

mod support;
const PASSWORD: &str = "disposable social HTTP fixture password";

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn email_disabled_allows_existing_and_new_unverified_accounts_without_claiming_verification()
{
    let pool = support::fresh_pool().await;
    MIGRATOR.run(&pool).await.unwrap();
    let mut values = ["DATABASE_URL", "CSRF_SECRET", "RATE_LIMIT_SECRET"]
        .into_iter()
        .map(|key| (key.into(), std::env::var(key).unwrap()))
        .collect::<HashMap<String, String>>();
    values.insert("EMAIL_ENABLED".into(), "false".into());
    let config = Arc::new(Config::from_values(&values).unwrap());
    let auth = Arc::new(BrowserAuth::new(pool.clone(), config).await.unwrap());
    let social = Social::new(pool.clone(), &values["RATE_LIMIT_SECRET"], 30)
        .unwrap()
        .with_email_enabled(false);
    let app = browser_router(pool.clone(), &auth, social.clone());
    let (owner, browser) = account(&pool, &auth, &app, "noemail", false).await;
    let (status, session, _) = browser
        .send(&app, "GET", "/browser/auth/session", Value::Null)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(session["emailEnabled"], false);
    assert_eq!(session["verified"], false);
    assert_eq!(
        browser
            .send(&app, "GET", "/browser/auth/settings", Value::Null)
            .await
            .1,
        json!({"emailEnabled":false})
    );
    let (status, post, _) = browser
        .send(
            &app,
            "POST",
            "/browser/v1/posts",
            json!({"body":"No email needed"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        browser
            .send(&app, "GET", "/browser/v1/feed", Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        browser
            .send(
                &app,
                "POST",
                "/browser/auth/reauthenticate",
                json!({"password":PASSWORD})
            )
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        browser
            .send(&app, "GET", "/browser/auth/keys", Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    for (path, body) in [
        ("forgot-password", json!({"email":"noemail@example.test"})),
        ("resend-verification", json!({})),
        ("verify-email", json!({"token":"unused-token"})),
        (
            "reset-password",
            json!({"token":"unused-token", "password":PASSWORD}),
        ),
    ] {
        let (status, result, _) = browser
            .send(&app, "POST", &format!("/browser/auth/{path}"), body)
            .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(result["error"]["code"], "email_disabled");
    }
    let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM delivery_jobs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(jobs, 0);
    let stored_verified: bool =
        sqlx::query_scalar("SELECT verified_at IS NOT NULL FROM users WHERE id=$1")
            .bind(owner)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!stored_verified);

    // Real integration adapter: disabling email changes only its mailbox check.
    sqlx::query("INSERT INTO api_keys(key_id,user_id,keyspace_id,label,masked_hint,permissions,expires_at) VALUES('key_noemail',$1,'ks_fixture','Fixture','masked',ARRAY['profiles.read'],now()+interval '1 day')")
        .bind(owner).execute(&pool).await.unwrap();
    let principal = json!({"version":"v1","type":"API_KEY","subject":owner,"identity":{"externalId":owner},
        "source":{"key":{"keyId":"key_noemail","keySpaceId":"ks_fixture","permissions":["profiles.read"]}}}).to_string();
    for (email_enabled, expected) in [(false, StatusCode::OK), (true, StatusCode::UNAUTHORIZED)] {
        let gateway = twitter_clone_api::http::gateway_router(
            pool.clone(),
            social.clone().with_email_enabled(email_enabled),
            Some("ks_fixture".into()),
        );
        let response = gateway
            .oneshot(
                Request::builder()
                    .uri("/v1/me")
                    .header("x-unkey-principal", &principal)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
    // Re-enabling email restores the requirement for the same existing session.
    let strict = Social::new(pool.clone(), &values["RATE_LIMIT_SECRET"], 30).unwrap();
    let strict_app = browser_router(pool.clone(), &auth, strict);
    assert_eq!(
        browser
            .send(&strict_app, "GET", "/browser/v1/feed", Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        browser
            .send(
                &app,
                "DELETE",
                &format!("/browser/v1/posts/{}", post["id"].as_str().unwrap()),
                Value::Null
            )
            .await
            .0,
        StatusCode::NO_CONTENT
    );
}

async fn setup(limit: u32) -> (PgPool, Arc<BrowserAuth>, Router) {
    let pool = support::fresh_pool().await;
    MIGRATOR.run(&pool).await.unwrap();
    let values = ["DATABASE_URL", "CSRF_SECRET", "RATE_LIMIT_SECRET"]
        .into_iter()
        .map(|k| (k.into(), std::env::var(k).unwrap()))
        .collect::<HashMap<_, _>>();
    let auth = Arc::new(
        BrowserAuth::new(
            pool.clone(),
            Arc::new(Config::from_values(&values).unwrap()),
        )
        .await
        .unwrap(),
    );
    let app = app(&pool, &auth, limit);
    (pool, auth, app)
}

fn app(pool: &PgPool, auth: &Arc<BrowserAuth>, limit: u32) -> Router {
    browser_router(
        pool.clone(),
        auth,
        Social::new(
            pool.clone(),
            &std::env::var("RATE_LIMIT_SECRET").unwrap(),
            limit,
        )
        .unwrap(),
    )
}

struct Browser {
    cookies: String,
    csrf: String,
}

impl Browser {
    async fn new(app: &Router, token: Option<&str>) -> Self {
        let mut browser = Self {
            cookies: token
                .map(|t| format!("twitterclone_dev_session={t}; "))
                .unwrap_or_default(),
            csrf: String::new(),
        };
        let response = app
            .clone()
            .oneshot(browser.request("GET", "/browser/auth/csrf", Value::Null))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let cookie = Cookie::parse(
            response.headers()[header::SET_COOKIE]
                .to_str()
                .unwrap()
                .to_owned(),
        )
        .unwrap();
        browser
            .cookies
            .push_str(&format!("{}={}", cookie.name(), cookie.value()));
        let body = to_bytes(response.into_body(), 16384).await.unwrap();
        browser.csrf = serde_json::from_slice::<Value>(&body).unwrap()["csrfToken"]
            .as_str()
            .unwrap()
            .into();
        browser
    }
    fn request(&self, method: &str, path: &str, body: Value) -> Request<Body> {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::COOKIE, &self.cookies)
            .header("x-csrf-token", &self.csrf)
            .header(header::ORIGIN, "http://localhost:3000")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo("192.0.2.1:1234".parse::<SocketAddr>().unwrap()));
        request
    }
    async fn send(
        &self,
        app: &Router,
        method: &str,
        path: &str,
        body: Value,
    ) -> (StatusCode, Value, HeaderMap) {
        let response = app
            .clone()
            .oneshot(self.request(method, path, body))
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        (
            status,
            if bytes.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&bytes).unwrap()
            },
            headers,
        )
    }
}

async fn account(
    pool: &PgPool,
    auth: &BrowserAuth,
    app: &Router,
    handle: &str,
    verified: bool,
) -> (Uuid, Browser) {
    let email = format!("{handle}@example.test");
    auth.accounts
        .register(&email, handle, PASSWORD)
        .await
        .unwrap();
    let grant = auth.accounts.login(&email, PASSWORD, None).await.unwrap();
    if verified {
        sqlx::query("UPDATE users SET verified_at = now() WHERE id = $1")
            .bind(grant.identity.id)
            .execute(pool)
            .await
            .unwrap();
    }
    (
        grant.identity.id,
        Browser::new(app, Some(grant.token.expose_secret())).await,
    )
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn browser_social_authorization_matrix_is_cookie_only_and_ownership_is_server_owned() {
    let (pool, auth, app) = setup(100).await;
    let (alice_id, alice) = account(&pool, &auth, &app, "Alice", true).await;
    let (bob_id, bob) = account(&pool, &auth, &app, "Bob", true).await;
    let (_, pending) = account(&pool, &auth, &app, "Pending", false).await;
    let anonymous = Browser::new(&app, None).await;
    let (status, post, _) = alice
        .send(&app, "POST", "/browser/v1/posts", json!({"body":"fixture"}))
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(post["authorId"], alice_id.to_string());
    let post_path = format!("/browser/v1/posts/{}", post["id"].as_str().unwrap());
    for viewer in [&anonymous, &pending, &alice, &bob] {
        for path in [
            "/browser/v1/profiles/Alice",
            "/browser/v1/profiles/Alice/posts",
            &post_path,
        ] {
            let (status, value, headers) = viewer.send(&app, "GET", path, Value::Null).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(headers[header::CACHE_CONTROL], "no-store");
            for private in ["@example.test", PASSWORD, "$argon2", "token_digest"] {
                assert!(!value.to_string().contains(private));
            }
        }
    }
    for (viewer, expected) in [
        (&anonymous, StatusCode::UNAUTHORIZED),
        (&pending, StatusCode::FORBIDDEN),
    ] {
        for (method, path, body) in [
            (
                "POST",
                "/browser/v1/posts".to_owned(),
                json!({"body":"denied"}),
            ),
            (
                "PATCH",
                "/browser/v1/me".to_owned(),
                json!({"displayName":"denied", "biography":""}),
            ),
            (
                "PUT",
                format!("/browser/v1/me/following/{bob_id}"),
                Value::Null,
            ),
            ("PUT", format!("{post_path}/like"), Value::Null),
            ("DELETE", post_path.clone(), Value::Null),
            ("GET", "/browser/v1/feed".to_owned(), Value::Null),
        ] {
            let result = viewer.send(&app, method, &path, body).await;
            assert_eq!(result.0, expected, "{method} {path}");
            assert!(result.1["requestId"].is_string());
        }
    }
    let mut forged = anonymous.request("POST", "/browser/v1/posts", json!({"body":"forged"}));
    forged
        .headers_mut()
        .insert("x-unkey-principal", alice_id.to_string().parse().unwrap());
    forged
        .headers_mut()
        .insert(header::AUTHORIZATION, "Bearer forged".parse().unwrap());
    assert_eq!(
        app.clone().oneshot(forged).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        bob.send(&app, "DELETE", &post_path, Value::Null).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        bob.send(
            &app,
            "PATCH",
            "/browser/v1/profiles/Alice",
            json!({"displayName":"spoof","biography":""})
        )
        .await
        .0,
        StatusCode::METHOD_NOT_ALLOWED
    );
    assert_eq!(
        bob.send(
            &app,
            "PATCH",
            "/browser/v1/me",
            json!({"displayName":"spoof","biography":"","userId":alice_id})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        bob.send(
            &app,
            "POST",
            "/browser/v1/posts",
            json!({"body":"spoof","authorId":alice_id})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        alice
            .send(&app, "GET", "/browser/v1/profiles/Alice", Value::Null)
            .await
            .1["postCount"],
        1
    );
    assert_eq!(
        alice.send(&app, "DELETE", &post_path, Value::Null).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        anonymous.send(&app, "GET", &post_path, Value::Null).await.0,
        StatusCode::NOT_FOUND
    );
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn browser_relationship_http_is_idempotent_and_reuses_one_write_budget() {
    let (pool, auth, first) = setup(3).await;
    let (_, alice) = account(&pool, &auth, &first, "Alice", true).await;
    let (bob_id, _) = account(&pool, &auth, &first, "Bob", true).await;
    let second = app(&pool, &auth, 3);
    let follow = format!("/browser/v1/me/following/{bob_id}");
    assert_eq!(
        alice.send(&first, "PUT", &follow, Value::Null).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        alice.send(&second, "PUT", &follow, Value::Null).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        alice
            .send(&first, "GET", "/browser/v1/profiles/Bob", Value::Null)
            .await
            .1["followerCount"],
        1
    );
    assert_eq!(
        alice.send(&first, "DELETE", &follow, Value::Null).await.0,
        StatusCode::NO_CONTENT
    );
    let result = alice
        .send(
            &second,
            "POST",
            "/browser/v1/posts",
            json!({"body":"limited"}),
        )
        .await;
    assert_eq!(result.0, StatusCode::TOO_MANY_REQUESTS);
    assert!(
        result.2[header::RETRY_AFTER]
            .to_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
            > 0
    );
    assert_eq!(
        alice
            .send(&first, "GET", "/browser/v1/profiles/Alice", Value::Null)
            .await
            .1["postCount"],
        0
    );
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn browser_social_mutations_require_origin_and_csrf_and_contract_matches_dtos() {
    let (pool, auth, app) = setup(100).await;
    let (alice_id, alice) = account(&pool, &auth, &app, "Alice", true).await;
    let (bob_id, bob) = account(&pool, &auth, &app, "Bob", true).await;
    for missing in [header::ORIGIN.as_str(), "x-csrf-token"] {
        let mut request = alice.request("POST", "/browser/v1/posts", json!({"body":"denied"}));
        request.headers_mut().remove(missing);
        assert_eq!(
            app.clone().oneshot(request).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }
    let updated = alice
        .send(
            &app,
            "PATCH",
            "/browser/v1/me",
            json!({"displayName":"Alice edited","biography":"<script>plain text</script>"}),
        )
        .await;
    assert_eq!(updated.0, StatusCode::OK);
    assert_eq!(updated.1["id"], alice_id.to_string());
    assert_eq!(updated.1["handle"], "Alice");
    assert_eq!(updated.1["biography"], "<script>plain text</script>");
    let post = bob
        .send(
            &app,
            "POST",
            "/browser/v1/posts",
            json!({"body":"from Bob"}),
        )
        .await
        .1;
    let path = format!("/browser/v1/posts/{}/like", post["id"].as_str().unwrap());
    for _ in 0..2 {
        assert_eq!(
            alice.send(&app, "PUT", &path, Value::Null).await.0,
            StatusCode::NO_CONTENT
        );
    }
    let read = alice
        .send(
            &app,
            "GET",
            path.strip_suffix("/like").unwrap(),
            Value::Null,
        )
        .await
        .1;
    assert_eq!(read["likeCount"], 1);
    assert_eq!(read["viewerLikes"], true);
    for _ in 0..2 {
        assert_eq!(
            alice.send(&app, "DELETE", &path, Value::Null).await.0,
            StatusCode::NO_CONTENT
        );
    }
    let following = format!("/browser/v1/me/following/{bob_id}");
    alice.send(&app, "PUT", &following, Value::Null).await;
    let feed = alice
        .send(&app, "GET", "/browser/v1/feed?limit=1", Value::Null)
        .await;
    assert_eq!(feed.0, StatusCode::OK);
    assert_eq!(feed.1["items"].as_array().unwrap().len(), 1);
    assert!(feed.1["nextCursor"].is_null());
    assert_eq!(
        alice
            .send(&app, "GET", "/browser/v1/feed?limit=101", Value::Null)
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        alice
            .send(&app, "GET", "/browser/v1/feed?cursor=invalid", Value::Null)
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        alice
            .send(
                &app,
                "PUT",
                &format!("/browser/v1/me/following/{alice_id}"),
                Value::Null
            )
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    pool.close().await;
}
