use super::*;
use crate::{accounts::Accounts, config::UnkeyConfig, database, tokens::BearerToken};
use axum::{Json, Router, extract::State, routing::post};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use tokio::sync::Notify;

const PASSWORD: &str = "isolated key management fixture password";
const SECRET: &str = "tc_fixture_one_time_secret_not_a_real_provider_key_1234";

struct Provider {
    creates: AtomicUsize,
    verifies: AtomicUsize,
    fail_delete: AtomicBool,
    entered: Notify,
    delay: Duration,
    requests: Mutex<Vec<serde_json::Value>>,
}
struct Fixture {
    pool: PgPool,
    accounts: Accounts,
    keys: Keys,
    provider: Arc<Provider>,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn setup(delay: Duration) -> Fixture {
    let parent_url = std::env::var("DATABASE_URL").expect("use tests/with_postgres.py");
    let parent = database::connect(&parent_url).await.unwrap();
    let name: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&parent)
        .await
        .unwrap();
    assert!(
        name.starts_with("twitterclone_test_"),
        "refusing non-test PostgreSQL"
    );
    let fresh = format!("twitterclone_test_{}", Uuid::new_v4().simple());
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {fresh}")))
        .execute(&parent)
        .await
        .unwrap();
    parent.close().await;
    let mut url = url::Url::parse(&parent_url).unwrap();
    url.set_path(&fresh);
    let pool = database::connect(url.as_str()).await.unwrap();
    database::MIGRATOR.run(&pool).await.unwrap();
    let values: HashMap<_, _> = ["DATABASE_URL", "CSRF_SECRET", "RATE_LIMIT_SECRET"]
        .into_iter()
        .map(|name| (name.into(), std::env::var(name).unwrap()))
        .collect();
    let mut keys = Keys::new(pool.clone(), &Config::from_values(&values).unwrap()).unwrap();
    let provider = Arc::new(Provider {
        creates: AtomicUsize::new(0),
        verifies: AtomicUsize::new(0),
        fail_delete: AtomicBool::new(false),
        entered: Notify::new(),
        delay,
        requests: Mutex::new(Vec::new()),
    });
    let router = Router::new()
        .route("/v2/keys.verifyKey", post(|State(state): State<Arc<Provider>>| async move {
            state.verifies.fetch_add(1, Ordering::SeqCst);
            StatusCode::SERVICE_UNAVAILABLE
        }))
        .route("/v2/keys.createKey", post(|State(state): State<Arc<Provider>>, Json(body): Json<serde_json::Value>| async move {
            state.requests.lock().unwrap().push(body);
            let index = state.creates.fetch_add(1, Ordering::SeqCst);
            state.entered.notify_one();
            tokio::time::sleep(state.delay).await;
            Json(serde_json::json!({"data":{"keyId":format!("key_fixture_{index}"),"key":SECRET}}))
        }))
        .route("/v2/keys.deleteKey", post(|State(state): State<Arc<Provider>>, Json(body): Json<serde_json::Value>| async move {
            state.requests.lock().unwrap().push(body);
            if state.fail_delete.load(Ordering::SeqCst) {
                (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"error":{"detail":"private fixture"}})))
            } else { (StatusCode::OK, Json(serde_json::json!({"data":{}}))) }
        })).with_state(provider.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    keys.management = Some(
        Management::fixture(
            &UnkeyConfig {
                management_root_key: "fixture-management-key-not-real".into(),
                api_id: "api_fixture".into(),
                keyspace_id: "ks_fixture".into(),
                requests_per_minute: 60,
            },
            endpoint,
        )
        .unwrap(),
    );
    let accounts = Accounts::new(pool.clone()).await.unwrap();
    Fixture {
        pool,
        accounts,
        keys,
        provider,
        server,
    }
}

async fn user(fixture: &Fixture, handle: &str, verified: bool) -> (ActiveSession, BearerToken) {
    let email = format!("{handle}@example.test");
    fixture
        .accounts
        .register(&email, handle, PASSWORD)
        .await
        .unwrap();
    let grant = fixture
        .accounts
        .login(&email, PASSWORD, None)
        .await
        .unwrap();
    if verified {
        sqlx::query("UPDATE users SET verified_at = now() WHERE id = $1")
            .bind(grant.identity.id)
            .execute(&fixture.pool)
            .await
            .unwrap();
    }
    let session = fixture
        .accounts
        .resolve_session(grant.token.expose_secret())
        .await
        .unwrap()
        .unwrap();
    (session, grant.token)
}
fn input(name: &str) -> KeyCreate {
    serde_json::from_value(serde_json::json!({"name":name})).unwrap()
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; use tests/with_postgres.py"]
async fn issuance_commits_metadata_reveals_only_once_and_lists_only_the_owner() {
    let fixture = setup(Duration::ZERO).await;
    let (alice, _) = user(&fixture, "alice", true).await;
    let (bob, _) = user(&fixture, "bob", true).await;
    let issued = fixture
        .keys
        .create(&alice, input("Automation"))
        .await
        .unwrap();
    assert!(issued.grant.expose_secret() == SECRET);
    assert_eq!(issued.metadata.status, "active");
    assert_eq!(
        issued.metadata.permissions,
        vec!["profiles.read", "posts.read", "feed.read"]
    );
    let list = fixture.keys.list(&alice).await.unwrap();
    assert!(list.integration_available);
    assert_eq!(list.items.len(), 1);
    let public = serde_json::to_string(&list).unwrap();
    assert!(!public.contains(SECRET));
    assert!(!public.contains("token_digest"));
    assert!(!public.contains("@example.test"));
    assert!(fixture.keys.list(&bob).await.unwrap().items.is_empty());
    let stored: String = sqlx::query_scalar("SELECT to_jsonb(api_keys)::text FROM api_keys")
        .fetch_one(&fixture.pool)
        .await
        .unwrap();
    assert!(!stored.contains(SECRET));
    assert!(!stored.contains("fixture-management-key-not-real"));
    assert_eq!(
        fixture.provider.requests.lock().unwrap()[0]["externalId"],
        alice.identity.id.to_string()
    );
    fixture.pool.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; use tests/with_postgres.py"]
async fn creation_rechecks_verification_recent_password_and_live_session_before_provider_call() {
    let fixture = setup(Duration::ZERO).await;
    let (pending, _) = user(&fixture, "pending", false).await;
    assert!(matches!(
        fixture.keys.create(&pending, input("No")).await,
        Err(KeyError::Unverified)
    ));
    let (alice, token) = user(&fixture, "alice", true).await;
    sqlx::query("UPDATE sessions SET password_authenticated_at = now() - interval '6 minutes' WHERE token_digest = $1")
        .bind(alice.binding()).execute(&fixture.pool).await.unwrap();
    assert!(matches!(
        fixture.keys.create(&alice, input("No")).await,
        Err(KeyError::RecentPasswordRequired)
    ));
    assert_eq!(fixture.provider.creates.load(Ordering::SeqCst), 0);
    fixture
        .accounts
        .reauthenticate(&alice, PASSWORD)
        .await
        .unwrap();
    fixture.keys.create(&alice, input("Yes")).await.unwrap();
    fixture
        .accounts
        .logout(token.expose_secret())
        .await
        .unwrap();
    assert!(matches!(
        fixture.keys.create(&alice, input("No")).await,
        Err(KeyError::Unauthenticated)
    ));
    assert!(matches!(
        fixture.keys.list(&alice).await,
        Err(KeyError::Unauthenticated)
    ));
    assert_eq!(fixture.provider.creates.load(Ordering::SeqCst), 1);
    fixture.pool.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; use tests/with_postgres.py"]
async fn revocation_is_owner_only_local_first_idempotent_and_cleanup_retries_durably() {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    let fixture = setup(Duration::ZERO).await;
    let (alice, _) = user(&fixture, "alice", true).await;
    let (bob, _) = user(&fixture, "bob", true).await;
    let issued = fixture
        .keys
        .create(&alice, input("Automation"))
        .await
        .unwrap();
    let id = &issued.metadata.key_id;
    let social = crate::social::Social::new(
        fixture.pool.clone(),
        &std::env::var("RATE_LIMIT_SECRET").unwrap(),
        30,
    )
    .unwrap();
    let gateway =
        crate::http::gateway_router(fixture.pool.clone(), social, Some("ks_fixture".into()));
    let principal = serde_json::json!({
        "version":"v1","type":"API_KEY","subject":alice.identity.id,
        "identity":{"externalId":alice.identity.id},
        "source":{"key":{"keyId":id,"keySpaceId":"ks_fixture",
            "permissions":["profiles.read","posts.read","feed.read"]}}
    })
    .to_string();
    let read = || {
        Request::builder()
            .uri("/v1/me")
            .header("x-unkey-principal", &principal)
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(
        gateway.clone().oneshot(read()).await.unwrap().status(),
        StatusCode::OK
    );
    assert_eq!(fixture.keys.revoke(&bob, id).await, Err(KeyError::NotFound));
    assert_eq!(
        fixture.keys.list(&alice).await.unwrap().items[0].status,
        "active"
    );
    fixture.provider.fail_delete.store(true, Ordering::SeqCst);
    fixture.keys.revoke(&alice, id).await.unwrap();
    fixture.keys.revoke(&alice, id).await.unwrap();
    assert_eq!(
        fixture.keys.list(&alice).await.unwrap().items[0].status,
        "revoked_pending_cleanup"
    );
    let first = fixture.keys.cleanup_batch().await.unwrap();
    assert_eq!((first.claimed, first.retried), (1, 1));
    assert_eq!(
        gateway.clone().oneshot(read()).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    let pending: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM delivery_jobs WHERE kind='unkey_revoke' AND attempts=1",
    )
    .fetch_one(&fixture.pool)
    .await
    .unwrap();
    assert_eq!(pending, 1);
    assert!(
        fixture.keys.list(&alice).await.unwrap().items[0]
            .provider_revoked_at
            .is_none()
    );
    fixture.provider.fail_delete.store(false, Ordering::SeqCst);
    sqlx::query("UPDATE delivery_jobs SET available_at=now() WHERE kind='unkey_revoke'")
        .execute(&fixture.pool)
        .await
        .unwrap();
    assert_eq!(fixture.keys.cleanup_batch().await.unwrap().completed, 1);
    let list = fixture.keys.list(&alice).await.unwrap();
    assert_eq!(list.items[0].status, "revoked");
    assert!(list.items[0].provider_revoked_at.is_some());
    assert_eq!(
        gateway.clone().oneshot(read()).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    fixture.keys.revoke(&alice, id).await.unwrap();
    assert_eq!(fixture.keys.cleanup_batch().await.unwrap().claimed, 0);
    assert_eq!(fixture.provider.creates.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture.provider.verifies.load(Ordering::SeqCst),
        0,
        "Rust must never spend another verification or auto-applied key limit"
    );
    assert_eq!(
        fixture.provider.requests.lock().unwrap().len(),
        3,
        "one creation, one failed soft deletion, one successful retry only"
    );
    fixture.pool.close().await;
    assert_eq!(
        gateway.oneshot(read()).await.unwrap().status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; use tests/with_postgres.py"]
async fn persistence_failure_never_returns_secret_and_persists_cleanup_for_unregistered_key() {
    let fixture = setup(Duration::ZERO).await;
    let (alice, _) = user(&fixture, "alice", true).await;
    sqlx::query("ALTER TABLE api_keys ADD CONSTRAINT fixture_refuse_label CHECK (label <> 'fail')")
        .execute(&fixture.pool)
        .await
        .unwrap();
    assert!(matches!(
        fixture.keys.create(&alice, input("fail")).await,
        Err(KeyError::Unavailable)
    ));
    assert!(fixture.keys.list(&alice).await.unwrap().items.is_empty());
    let jobs: i64 =
        sqlx::query_scalar("SELECT count(*) FROM delivery_jobs WHERE kind='unkey_revoke'")
            .fetch_one(&fixture.pool)
            .await
            .unwrap();
    assert_eq!(jobs, 1);
    assert_eq!(fixture.keys.cleanup_batch().await.unwrap().completed, 1);
    fixture.pool.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; use tests/with_postgres.py"]
async fn reset_serializes_with_inflight_issuance_and_revokes_the_new_key_and_session() {
    let fixture = setup(Duration::from_millis(50)).await;
    let (alice, token) = user(&fixture, "alice", true).await;
    let reset = BearerToken::generate().unwrap();
    sqlx::query("INSERT INTO account_tokens (token_digest,user_id,purpose,expires_at) VALUES ($1,$2,'password_reset',now()+interval '30 minutes')")
        .bind(reset.digest()).bind(alice.identity.id).execute(&fixture.pool).await.unwrap();
    let (created, reset_result) = tokio::join!(
        fixture.keys.create(&alice, input("Racing issuance")),
        async {
            fixture.provider.entered.notified().await;
            fixture
                .accounts
                .reset_password(reset.expose_secret(), "replacement key fixture password")
                .await
        },
    );
    let created = created.unwrap();
    reset_result.unwrap();
    let revoked: bool =
        sqlx::query_scalar("SELECT revoked_at IS NOT NULL FROM api_keys WHERE key_id=$1")
            .bind(&created.metadata.key_id)
            .fetch_one(&fixture.pool)
            .await
            .unwrap();
    assert!(revoked);
    assert!(
        fixture
            .accounts
            .resolve_session(token.expose_secret())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(fixture.keys.cleanup_batch().await.unwrap().completed, 1);
    fixture.pool.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; use tests/with_postgres.py"]
async fn browser_key_http_reveals_once_uses_no_store_rejects_injection_and_redacts_persistence_failure()
 {
    use axum::{
        body::{Body, to_bytes},
        extract::ConnectInfo,
        http::{Request, header},
    };
    use tower::ServiceExt;
    let fixture = setup(Duration::ZERO).await;
    let (_, token) = user(&fixture, "alice", true).await;
    let values: HashMap<_, _> = ["DATABASE_URL", "CSRF_SECRET", "RATE_LIMIT_SECRET"]
        .into_iter()
        .map(|name| (name.into(), std::env::var(name).unwrap()))
        .collect();
    let mut auth = crate::browser::BrowserAuth::new(
        fixture.pool.clone(),
        Arc::new(Config::from_values(&values).unwrap()),
    )
    .await
    .unwrap();
    auth.keys = fixture.keys.clone();
    let auth = Arc::new(auth);
    let social = crate::social::Social::new(
        fixture.pool.clone(),
        &std::env::var("RATE_LIMIT_SECRET").unwrap(),
        30,
    )
    .unwrap();
    let app = crate::http::browser_router(fixture.pool.clone(), &auth, social);
    let mut cookies = format!("twitterclone_dev_session={}", token.expose_secret());
    let build = |method: &str, path: &str, cookies: &str, csrf: &str, value: serde_json::Value| {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::COOKIE, cookies)
            .header(header::ORIGIN, "http://localhost:3000")
            .header("x-csrf-token", csrf)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(value.to_string()))
            .unwrap();
        request.extensions_mut().insert(ConnectInfo(
            "192.0.2.1:1000".parse::<std::net::SocketAddr>().unwrap(),
        ));
        request
    };
    let bootstrap = app
        .clone()
        .oneshot(build(
            "GET",
            "/browser/auth/csrf",
            &cookies,
            "",
            serde_json::Value::Null,
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
    let csrf: serde_json::Value =
        serde_json::from_slice(&to_bytes(bootstrap.into_body(), 16384).await.unwrap()).unwrap();
    let csrf = csrf["csrfToken"].as_str().unwrap();
    let issued = app
        .clone()
        .oneshot(build(
            "POST",
            "/browser/auth/keys",
            &cookies,
            csrf,
            serde_json::json!({"name":"Fixture"}),
        ))
        .await
        .unwrap();
    assert_eq!(issued.status(), StatusCode::CREATED);
    assert_eq!(issued.headers()[header::CACHE_CONTROL], "no-store");
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(issued.into_body(), 16384).await.unwrap()).unwrap();
    assert!(body["key"].as_str().unwrap() == SECRET);
    let id = body["metadata"]["keyId"].as_str().unwrap();
    for value in [
        serde_json::json!({"name":"spoof","ownerId":Uuid::new_v4()}),
        serde_json::json!({"name":"spoof","permissions":["admin"]}),
        serde_json::json!({"name":"spoof","recoverable":true}),
    ] {
        let response = app
            .clone()
            .oneshot(build("POST", "/browser/auth/keys", &cookies, csrf, value))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
    assert_eq!(fixture.provider.creates.load(Ordering::SeqCst), 1);
    let listed = app
        .clone()
        .oneshot(build(
            "GET",
            "/browser/auth/keys",
            &cookies,
            csrf,
            serde_json::Value::Null,
        ))
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    let bytes = to_bytes(listed.into_body(), 16384).await.unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains(SECRET));
    let revoked = app
        .clone()
        .oneshot(build(
            "DELETE",
            &format!("/browser/auth/keys/{id}"),
            &cookies,
            csrf,
            serde_json::Value::Null,
        ))
        .await
        .unwrap();
    assert_eq!(revoked.status(), StatusCode::NO_CONTENT);
    sqlx::query("ALTER TABLE api_keys ADD CONSTRAINT fixture_refuse_label CHECK (label <> 'fail')")
        .execute(&fixture.pool)
        .await
        .unwrap();
    let failure = app
        .oneshot(build(
            "POST",
            "/browser/auth/keys",
            &cookies,
            csrf,
            serde_json::json!({"name":"fail"}),
        ))
        .await
        .unwrap();
    assert_eq!(failure.status(), StatusCode::SERVICE_UNAVAILABLE);
    let bytes = to_bytes(failure.into_body(), 16384).await.unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains(SECRET));
    fixture.pool.close().await;
}
