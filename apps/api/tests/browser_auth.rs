use axum::{
    Router,
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{Request, StatusCode, header},
    response::Response,
};
use cookie::Cookie;
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{
    collections::{BTreeMap, HashMap},
    net::SocketAddr,
    sync::Arc,
};
use tower::ServiceExt;
use twitter_clone_api::{
    browser::BrowserAuth, config::Config, database::MIGRATOR, http::browser_router, social::Social,
    tokens::BearerToken,
};

mod support;
const PASSWORD: &str = "disposable browser fixture password";
const ORIGIN: &str = "http://localhost:3000";

async fn setup() -> (PgPool, Arc<BrowserAuth>, Router) {
    let pool = support::fresh_pool().await;
    MIGRATOR.run(&pool).await.unwrap();
    let auth = new_auth(pool.clone()).await;
    let app = browser_router(
        pool.clone(),
        &auth,
        Social::new(
            pool.clone(),
            &std::env::var("RATE_LIMIT_SECRET").unwrap(),
            30,
        )
        .unwrap(),
    );
    (pool, auth, app)
}

async fn new_auth(pool: PgPool) -> Arc<BrowserAuth> {
    let values: HashMap<_, _> = ["DATABASE_URL", "CSRF_SECRET", "RATE_LIMIT_SECRET"]
        .into_iter()
        .map(|key| (key.to_owned(), std::env::var(key).unwrap()))
        .collect();
    Arc::new(
        BrowserAuth::new(pool, Arc::new(Config::from_values(&values).unwrap()))
            .await
            .unwrap(),
    )
}

#[derive(Default, Clone)]
struct Browser {
    cookies: BTreeMap<String, String>,
    csrf: String,
}

impl Browser {
    fn request(&self, path: &str, body: Value) -> Request<Body> {
        let mut request = Request::builder()
            .method("POST")
            .uri(path)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ORIGIN, ORIGIN)
            .header("x-csrf-token", &self.csrf)
            .header(header::COOKIE, self.cookie_header())
            .body(Body::from(body.to_string()))
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo("192.0.2.1:1234".parse::<SocketAddr>().unwrap()));
        request
    }

    fn cookie_header(&self) -> String {
        self.cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    async fn absorb(&mut self, response: Response) -> (StatusCode, Value) {
        let status = response.status();
        for value in response.headers().get_all(header::SET_COOKIE) {
            let cookie = Cookie::parse(value.to_str().unwrap()).unwrap();
            if cookie.value().is_empty() {
                self.cookies.remove(cookie.name());
            } else {
                self.cookies
                    .insert(cookie.name().to_owned(), cookie.value().to_owned());
            }
        }
        let body = to_bytes(response.into_body(), 16384).await.unwrap();
        let value: Value = if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&body).unwrap()
        };
        if let Some(token) = value["csrfToken"].as_str() {
            self.csrf = token.to_owned();
        }
        (status, value)
    }

    async fn bootstrap(&mut self, app: &Router) {
        let mut request = self.request("/browser/auth/csrf", Value::Null);
        *request.method_mut() = axum::http::Method::GET;
        request.headers_mut().remove(header::ORIGIN); // native same-origin GET need not send Origin
        assert_eq!(
            self.absorb(app.clone().oneshot(request).await.unwrap())
                .await
                .0,
            StatusCode::OK
        );
    }

    async fn post(&mut self, app: &Router, path: &str, body: Value) -> (StatusCode, Value) {
        let request = self.request(path, body);
        self.absorb(app.clone().oneshot(request).await.unwrap())
            .await
    }

    async fn login(&mut self, app: &Router, email: &str) {
        self.bootstrap(app).await;
        assert_eq!(
            self.post(
                app,
                "/browser/auth/login",
                json!({"email": email, "password": PASSWORD})
            )
            .await
            .0,
            StatusCode::OK
        );
    }
}

async fn register(auth: &BrowserAuth) {
    auth.accounts
        .register("fixture@example.test", "fixture", PASSWORD)
        .await
        .unwrap();
}

async fn count(pool: &PgPool, query: &'static str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(query))
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn csrf_and_exact_origin_reject_auth_mutations_before_side_effects() {
    let (pool, auth, app) = setup().await;
    register(&auth).await;
    let mut browser = Browser::default();
    browser.bootstrap(&app).await;
    let mut attacker = Browser::default();
    attacker.bootstrap(&app).await;
    for path in [
        "/browser/auth/register",
        "/browser/auth/login",
        "/browser/auth/verify-email",
        "/browser/auth/forgot-password",
        "/browser/auth/reset-password",
        "/browser/auth/logout",
        "/browser/auth/resend-verification",
        "/browser/auth/reauthenticate",
    ] {
        for variant in 0..9 {
            let mut request = browser.request(
                path,
                json!({"email": "other@example.test",
                "handle": "other", "password": PASSWORD, "token": "private-token"}),
            );
            match variant {
                0 => {
                    request.headers_mut().remove(header::ORIGIN);
                }
                1 => {
                    request
                        .headers_mut()
                        .insert(header::ORIGIN, "https://hostile.example".parse().unwrap());
                }
                2 => {
                    request
                        .headers_mut()
                        .insert(header::ORIGIN, "null".parse().unwrap());
                }
                3 => {
                    request
                        .headers_mut()
                        .insert(header::ORIGIN, "http://localhost:3000/".parse().unwrap());
                }
                4 => {
                    request.headers_mut().remove("x-csrf-token");
                }
                5 => {
                    request
                        .headers_mut()
                        .insert("x-csrf-token", attacker.csrf.parse().unwrap());
                }
                6 => {
                    request.headers_mut().remove(header::COOKIE);
                }
                7 => {
                    request
                        .headers_mut()
                        .append(header::ORIGIN, ORIGIN.parse().unwrap());
                }
                8 => {
                    request
                        .headers_mut()
                        .append("x-csrf-token", browser.csrf.parse().unwrap());
                }
                _ => unreachable!(),
            }
            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(
                response.status(),
                StatusCode::FORBIDDEN,
                "{path} variant {variant}"
            );
            let (_, value) = browser.absorb(response).await;
            assert_eq!(value["error"]["code"], "csrf_rejected");
            assert!(!value.to_string().contains("private-token"));
        }
    }
    assert_eq!(count(&pool, "SELECT count(*) FROM users").await, 1);
    assert_eq!(count(&pool, "SELECT count(*) FROM sessions").await, 0);
    assert_eq!(count(&pool, "SELECT count(*) FROM delivery_jobs").await, 1);
    assert_eq!(
        count(&pool, "SELECT count(*) FROM rate_limit_buckets").await,
        0
    );
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn session_rotation_binds_csrf_and_forged_identity_never_authenticates() {
    let (pool, auth, app) = setup().await;
    register(&auth).await;
    let mut browser = Browser::default();
    browser.bootstrap(&app).await;
    let anonymous = browser.clone();
    browser.login(&app, "fixture@example.test").await;
    let mut replay = anonymous.clone();
    replay.cookies.insert(
        "twitterclone_dev_session".into(),
        browser.cookies["twitterclone_dev_session"].clone(),
    );
    assert_eq!(
        replay
            .post(&app, "/browser/auth/logout", Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        anonymous
            .clone()
            .post(&app, "/browser/auth/resend-verification", Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let mut forged = anonymous.request("/browser/auth/session", Value::Null);
    *forged.method_mut() = axum::http::Method::GET;
    forged.headers_mut().insert(
        "x-unkey-principal",
        "forged-verified-owner".parse().unwrap(),
    );
    forged
        .headers_mut()
        .insert(header::AUTHORIZATION, "Bearer forged".parse().unwrap());
    assert_eq!(
        app.clone().oneshot(forged).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    // A pair signed for one live session cannot be transplanted into another.
    let first = browser.clone();
    browser.login(&app, "fixture@example.test").await;
    let mut transplanted = first.clone();
    transplanted.cookies.insert(
        "twitterclone_dev_session".into(),
        browser.cookies["twitterclone_dev_session"].clone(),
    );
    assert_eq!(
        transplanted
            .post(&app, "/browser/auth/logout", Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        browser
            .post(&app, "/browser/auth/resend-verification", Value::Null)
            .await
            .0,
        StatusCode::ACCEPTED
    );
    let old_session = browser.cookies["twitterclone_dev_session"].clone();
    assert_eq!(
        browser
            .post(&app, "/browser/auth/logout", Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    assert!(!browser.cookies.contains_key("twitterclone_dev_session"));
    assert!(
        auth.accounts
            .resolve_session(&old_session)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        browser
            .post(&app, "/browser/auth/logout", Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn unverified_session_cannot_reauthenticate_and_verification_requires_protected_post() {
    let (pool, auth, app) = setup().await;
    register(&auth).await;
    let token = BearerToken::generate().unwrap();
    sqlx::query(
        "INSERT INTO account_tokens (token_digest, user_id, purpose, expires_at) \
        SELECT $1, id, 'email_verification', now() + interval '1 hour' FROM users",
    )
    .bind(token.digest())
    .execute(&pool)
    .await
    .unwrap();
    let mut browser = Browser::default();
    browser.login(&app, "fixture@example.test").await;
    sqlx::query("UPDATE sessions SET password_authenticated_at = now() - interval '6 minutes'")
        .execute(&pool)
        .await
        .unwrap();
    let mut request = browser.request(
        "/browser/auth/reauthenticate",
        json!({"password": PASSWORD}),
    );
    request
        .headers_mut()
        .insert("x-unkey-principal", "verified=true".parse().unwrap());
    assert_eq!(
        app.clone().oneshot(request).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(count(&pool, "SELECT count(*) FROM sessions WHERE password_authenticated_at > now() - interval '5 minutes'").await, 0);
    let mut request = browser.request(
        "/browser/auth/verify-email",
        json!({"token": token.expose_secret()}),
    );
    *request.method_mut() = axum::http::Method::GET;
    assert_eq!(
        app.clone().oneshot(request).await.unwrap().status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM users WHERE verified_at IS NOT NULL"
        )
        .await,
        0
    );
    assert_eq!(
        browser
            .post(
                &app,
                "/browser/auth/verify-email",
                json!({"token": token.expose_secret()})
            )
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        browser
            .post(
                &app,
                "/browser/auth/reauthenticate",
                json!({"password": PASSWORD})
            )
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn auth_throttles_are_shared_normalized_and_ignore_public_forwarding_headers() {
    let (pool, auth, first) = setup().await;
    register(&auth).await;
    let other_auth = new_auth(pool.clone()).await;
    let second = browser_router(
        pool.clone(),
        &other_auth,
        Social::new(
            pool.clone(),
            &std::env::var("RATE_LIMIT_SECRET").unwrap(),
            30,
        )
        .unwrap(),
    );
    let mut browser = Browser::default();
    browser.bootstrap(&first).await;
    for attempt in 0..6 {
        let app = if attempt % 2 == 0 { &first } else { &second };
        let mut request = browser.request("/browser/auth/login", json!({
            "email": if attempt % 2 == 0 { " FIXTURE@EXAMPLE.TEST " } else { "fixture@example.test" },
            "password": "wrong fixture password"}));
        request.headers_mut().insert(
            "x-forwarded-for",
            format!("198.51.100.{attempt}").parse().unwrap(),
        );
        request.headers_mut().insert(
            "forwarded",
            format!("for=198.51.100.{attempt}").parse().unwrap(),
        );
        let response = app.clone().oneshot(request).await.unwrap();
        if attempt < 5 {
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        } else {
            assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
            assert!(
                response.headers()[header::RETRY_AFTER]
                    .to_str()
                    .unwrap()
                    .parse::<u64>()
                    .unwrap()
                    > 0
            );
        }
    }
    assert_eq!(count(&pool, "SELECT count(*) FROM sessions").await, 0);
    for attempt in 0..51 {
        let response = first.clone().oneshot(browser.request("/browser/auth/login",
            json!({"email": format!("unknown{attempt}@example.test"), "password": "invalid-short"}))).await.unwrap();
        // Prior login attempts consumed six of the 50 IP attempts. The IP limit
        // applies before credential validation/hash work, even with changing email.
        assert_eq!(
            response.status(),
            if attempt < 44 {
                StatusCode::UNAUTHORIZED
            } else {
                StatusCode::TOO_MANY_REQUESTS
            }
        );
    }
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn registration_reset_resend_and_reauthentication_have_bounded_attempts() {
    let (pool, auth, app) = setup().await;
    register(&auth).await;
    let mut browser = Browser::default();
    browser.bootstrap(&app).await;
    for i in 0..6 {
        assert_eq!(browser.post(&app, "/browser/auth/register",
            json!({"email": format!("new{i}@example.test"), "handle": "invalid!", "password": PASSWORD})).await.0,
            if i < 5 { StatusCode::UNPROCESSABLE_ENTITY } else { StatusCode::TOO_MANY_REQUESTS });
    }
    for i in 0..4 {
        assert_eq!(
            browser
                .post(
                    &app,
                    "/browser/auth/forgot-password",
                    json!({"email": " UNKNOWN@EXAMPLE.TEST "})
                )
                .await
                .0,
            if i < 3 {
                StatusCode::ACCEPTED
            } else {
                StatusCode::TOO_MANY_REQUESTS
            }
        );
    }
    browser.login(&app, "fixture@example.test").await;
    for i in 0..4 {
        assert_eq!(
            browser
                .post(&app, "/browser/auth/resend-verification", Value::Null)
                .await
                .0,
            if i < 3 {
                StatusCode::ACCEPTED
            } else {
                StatusCode::TOO_MANY_REQUESTS
            }
        );
    }
    sqlx::query("UPDATE users SET verified_at = now()")
        .execute(&pool)
        .await
        .unwrap();
    for i in 0..6 {
        assert_eq!(
            browser
                .post(
                    &app,
                    "/browser/auth/reauthenticate",
                    json!({"password": "wrong fixture password"})
                )
                .await
                .0,
            if i < 5 {
                StatusCode::UNAUTHORIZED
            } else {
                StatusCode::TOO_MANY_REQUESTS
            }
        );
    }
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn unavailable_throttle_storage_denies_registration() {
    let (pool, _, app) = setup().await;
    let mut browser = Browser::default();
    browser.bootstrap(&app).await;
    pool.close().await;
    assert_eq!(
        browser
            .post(
                &app,
                "/browser/auth/register",
                json!({"email": "fixture@example.test", "handle": "fixture", "password": PASSWORD})
            )
            .await
            .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn reset_http_is_generic_requires_csrf_and_forces_fresh_login() {
    let (pool, auth, app) = setup().await;
    register(&auth).await;
    let mut browser = Browser::default();
    browser.bootstrap(&app).await;
    for email in [
        "fixture@example.test",
        "unknown@example.test",
        "invalid-private-email",
    ] {
        assert_eq!(
            browser
                .post(
                    &app,
                    "/browser/auth/forgot-password",
                    json!({"email": email})
                )
                .await,
            (StatusCode::ACCEPTED, Value::Null)
        );
    }
    browser.login(&app, "fixture@example.test").await;
    let old_session = browser.cookies["twitterclone_dev_session"].clone();
    let reset = BearerToken::generate().unwrap();
    sqlx::query(
        "INSERT INTO account_tokens (token_digest, user_id, purpose, expires_at) \
        SELECT $1, id, 'password_reset', now() + interval '30 minutes' FROM users",
    )
    .bind(reset.digest())
    .execute(&pool)
    .await
    .unwrap();
    let input =
        json!({"token": reset.expose_secret(), "password": "new disposable HTTP fixture password"});
    let mut cross_site = browser.request("/browser/auth/reset-password", input.clone());
    cross_site
        .headers_mut()
        .insert(header::ORIGIN, "https://hostile.example".parse().unwrap());
    assert_eq!(
        app.clone().oneshot(cross_site).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    assert!(
        auth.accounts
            .resolve_session(&old_session)
            .await
            .unwrap()
            .is_some()
    );
    let (status, response) = browser
        .post(&app, "/browser/auth/reset-password", input.clone())
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!response.to_string().contains(reset.expose_secret()));
    assert!(!response.to_string().contains("password"));
    assert!(!browser.cookies.contains_key("twitterclone_dev_session"));
    assert!(
        auth.accounts
            .resolve_session(&old_session)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        browser
            .post(&app, "/browser/auth/reset-password", input)
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn documented_account_contract_preserves_public_shapes_and_redacts_failed_input() {
    let (pool, _, app) = setup().await;
    let mut browser = Browser::default();
    browser.bootstrap(&app).await;
    let registration =
        json!({"email": "fixture@example.test", "handle": "Fixture", "password": PASSWORD});
    assert_eq!(
        browser
            .post(&app, "/browser/auth/register", registration.clone())
            .await,
        (StatusCode::ACCEPTED, Value::Null)
    );
    assert_eq!(
        browser
            .post(&app, "/browser/auth/register", registration)
            .await,
        (StatusCode::ACCEPTED, Value::Null)
    );
    let mut login = browser.request(
        "/browser/auth/login",
        json!({"email": "fixture@example.test", "password": PASSWORD}),
    );
    let response = app.clone().oneshot(login).await.unwrap();
    assert_eq!(
        response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .count(),
        2
    );
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(response.headers()[header::REFERRER_POLICY], "no-referrer");
    let (status, value) = browser.absorb(response).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value.as_object().unwrap().len(), 1);
    assert!(value["csrfToken"].is_string());
    login = browser.request("/browser/auth/session", Value::Null);
    *login.method_mut() = axum::http::Method::GET;
    let (_, identity) = browser
        .absorb(app.clone().oneshot(login).await.unwrap())
        .await;
    assert_eq!(identity.as_object().unwrap().len(), 4);
    assert_eq!(identity["handle"], "Fixture");
    assert_eq!(identity["verified"], false);
    assert_eq!(identity["recentPasswordAuth"], true);
    assert!(uuid::Uuid::parse_str(identity["id"].as_str().unwrap()).is_ok());
    for private in [
        "fixture@example.test",
        PASSWORD,
        "$argon2",
        "digest",
        "csrfToken",
    ] {
        assert!(!identity.to_string().contains(private));
    }
    let mut failures = Vec::new();
    for email in ["fixture@example.test", "unknown@example.test"] {
        let (status, body) = browser
            .post(
                &app,
                "/browser/auth/login",
                json!({"email": email, "password": "private wrong fixture password"}),
            )
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        failures.push(body["error"].clone());
        for private in [email, "private wrong fixture password", "$argon2"] {
            assert!(!body.to_string().contains(private));
        }
    }
    assert_eq!(failures[0], failures[1]);
    for (payload, content_type, expected) in [
        (
            "{private-malformed-json",
            "application/json",
            StatusCode::BAD_REQUEST,
        ),
        (
            "{\"token\":\"private-token\",\"email\":\"private@example.test\"}",
            "application/json",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "private-body",
            "text/plain",
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
        ),
    ] {
        let mut request = browser.request("/browser/auth/verify-email", Value::Null);
        request
            .headers_mut()
            .insert(header::CONTENT_TYPE, content_type.parse().unwrap());
        *request.body_mut() = Body::from(payload);
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), expected);
        let request_id = response.headers()["x-request-id"]
            .to_str()
            .unwrap()
            .to_owned();
        let (_, body) = browser.absorb(response).await;
        assert_eq!(body["requestId"], request_id);
        assert!(!body.to_string().contains("private"));
    }
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn real_process_logs_do_not_include_account_request_secrets() {
    use std::{
        io::{Read, Write},
        net::TcpStream,
        process::{Child, Command, Stdio},
        time::Duration,
    };
    // A child is always stopped, even if an assertion fails.
    struct Process(Option<Child>);
    impl Drop for Process {
        fn drop(&mut self) {
            if let Some(child) = &mut self.0 {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }
    fn request(port: u16, path: &str, body: &str) -> std::io::Result<String> {
        let mut stream = TcpStream::connect_timeout(
            &format!("127.0.0.1:{port}").parse().unwrap(),
            Duration::from_millis(100),
        )?;
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        write!(
            stream,
            "POST {path} HTTP/1.1\r\nHost: localhost\r\nOrigin: https://hostile.example\r\nContent-Type: application/json\r\nX-Unkey-Principal: private-principal-fixture\r\nCookie: twitterclone_dev_session=private-cookie-fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )?;
        let mut response = String::new();
        stream.read_to_string(&mut response)?;
        Ok(response)
    }
    let pool = support::fresh_pool().await;
    MIGRATOR.run(&pool).await.unwrap();
    let name: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&pool)
        .await
        .unwrap();
    let mut database = url::Url::parse(&std::env::var("DATABASE_URL").unwrap()).unwrap();
    database.set_path(&name);
    let browser_port = free_port();
    let mut gateway_port = free_port();
    while browser_port == gateway_port {
        gateway_port = free_port();
    }
    let mut process = Process(Some(
        Command::new(env!("CARGO_BIN_EXE_twitter-clone-api"))
            .env_clear()
            .env("DATABASE_URL", database.as_str())
            .env("CSRF_SECRET", std::env::var("CSRF_SECRET").unwrap())
            .env(
                "RATE_LIMIT_SECRET",
                std::env::var("RATE_LIMIT_SECRET").unwrap(),
            )
            .env("BROWSER_BIND", format!("127.0.0.1:{browser_port}"))
            .env("GATEWAY_BIND", format!("127.0.0.1:{gateway_port}"))
            .env("SMTP_HOST", "127.0.0.1")
            .env("SMTP_PORT", "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    ));
    let body = r#"{"email":"private-email@example.test","password":"private-password-fixture","token":"private-token-fixture"}"#;
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        match request(
            browser_port,
            "/browser/auth/login?token=private-query-fixture",
            body,
        ) {
            Ok(response) => {
                assert!(response.starts_with("HTTP/1.1 403"));
                assert!(!response.contains("private-"));
                break;
            }
            Err(_) if std::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(25)).await
            }
            Err(_) => panic!("account fixture process did not serve within deadline"),
        }
    }
    let mut child = process.0.take().unwrap();
    child.kill().unwrap();
    let output = child.wait_with_output().unwrap();
    let logs = [output.stdout, output.stderr].concat();
    let logs = String::from_utf8_lossy(&logs);
    assert!(logs.contains("Browser API listening"));
    assert!(!logs.contains("private-"));
    assert!(!logs.contains(database.as_str()));
    assert!(!logs.contains(&std::env::var("CSRF_SECRET").unwrap()));
    assert!(!logs.contains(&std::env::var("RATE_LIMIT_SECRET").unwrap()));
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn key_management_without_provider_setup_lists_metadata_but_never_issues_fake_secrets() {
    let (pool, auth, app) = setup().await;
    register(&auth).await;
    let mut browser = Browser::default();
    browser.login(&app, "fixture@example.test").await;
    let mut request = browser.request("/browser/auth/keys", Value::Null);
    *request.method_mut() = axum::http::Method::GET;
    assert_eq!(
        app.clone().oneshot(request).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    sqlx::query("UPDATE users SET verified_at=now()")
        .execute(&pool)
        .await
        .unwrap();
    let mut request = browser.request("/browser/auth/keys", Value::Null);
    *request.method_mut() = axum::http::Method::GET;
    let (status, list) = browser
        .absorb(app.clone().oneshot(request).await.unwrap())
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["integrationAvailable"], false);
    assert_eq!(list["items"], json!([]));
    let (status, error) = browser
        .post(&app, "/browser/auth/keys", json!({"name":"No provider"}))
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error["error"]["code"], "integration_unavailable");
    assert!(error.get("key").is_none());
    pool.close().await;
}
