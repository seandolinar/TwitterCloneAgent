//! Unkey API Management only. Gateway authentication must not verify keys here
//! a second time or double-count the provider's per-key limit.
use crate::config::UnkeyConfig;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use reqwest::{
    Client, StatusCode,
    header::{AUTHORIZATION, HeaderMap, HeaderValue},
};
use serde::{Deserialize, Serialize};
use std::{error::Error, fmt, time::Duration};
use uuid::Uuid;
use zeroize::Zeroizing;

const ENDPOINT: &str = "https://api.unkey.com/";
const RESPONSE_BYTES: usize = 16 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Permission {
    #[serde(rename = "profiles.read")]
    ProfilesRead,
    #[serde(rename = "profiles.write")]
    ProfilesWrite,
    #[serde(rename = "posts.read")]
    PostsRead,
    #[serde(rename = "posts.write")]
    PostsWrite,
    #[serde(rename = "feed.read")]
    FeedRead,
    #[serde(rename = "follows.write")]
    FollowsWrite,
    #[serde(rename = "likes.write")]
    LikesWrite,
}

fn read_only() -> Vec<Permission> {
    vec![
        Permission::ProfilesRead,
        Permission::PostsRead,
        Permission::FeedRead,
    ]
}
fn ninety_days() -> u16 {
    90
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KeyCreate {
    pub name: String,
    #[serde(default = "read_only")]
    pub permissions: Vec<Permission>,
    #[serde(default = "ninety_days")]
    pub expires_in_days: u16,
}

impl KeyCreate {
    pub fn validate(&self) -> Result<(), UnkeyError> {
        let mut permissions = self.permissions.clone();
        permissions.sort_unstable();
        permissions.dedup();
        if self.name.trim().is_empty()
            || self.name.chars().count() > 100
            || self.name.chars().any(char::is_control)
            || !(1..=365).contains(&self.expires_in_days)
            || self.permissions.is_empty()
            || permissions.len() != self.permissions.len()
        {
            return Err(UnkeyError::InvalidInput);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnkeyError {
    InvalidConfiguration,
    InvalidInput,
    Unavailable,
    AmbiguousCreate,
}

impl fmt::Display for UnkeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidConfiguration => "Unkey management configuration is invalid.",
            Self::InvalidInput => {
                "Choose a name, supported permissions, and an expiry within 365 days."
            }
            Self::Unavailable => "Key management is temporarily unavailable.",
            Self::AmbiguousCreate => "Key creation status is uncertain. Reconcile before retrying.",
        })
    }
}
impl Error for UnkeyError {}

// Intentionally not Debug/Serialize: the raw secret has only a one-time boundary.
pub struct CreatedKey {
    pub key_id: String,
    pub expires_at: DateTime<Utc>,
    secret: Zeroizing<String>,
}

impl CreatedKey {
    pub fn expose_secret(&self) -> &str {
        &self.secret
    }
    pub fn masked_hint(&self) -> String {
        format!("tc_…{}", &self.secret[self.secret.len() - 4..])
    }
}

// The Client contains a sensitive Authorization header and has no diagnostic
// implementation. Runtime callers cannot choose a provider URL.
#[derive(Clone)]
pub struct Management {
    client: Client,
    endpoint: url::Url,
    api_id: String,
    keyspace_id: String,
    requests_per_minute: u32,
}

impl Management {
    pub fn new(config: &UnkeyConfig) -> Result<Self, UnkeyError> {
        Self::build(
            config,
            url::Url::parse(ENDPOINT).expect("fixed provider URL"),
            REQUEST_TIMEOUT,
        )
    }

    fn build(
        config: &UnkeyConfig,
        endpoint: url::Url,
        timeout: Duration,
    ) -> Result<Self, UnkeyError> {
        if !identifier(&config.api_id, "api_")
            || !identifier(&config.keyspace_id, "ks_")
            || config.management_root_key.trim().is_empty()
            || config.requests_per_minute == 0
        {
            return Err(UnkeyError::InvalidConfiguration);
        }
        let mut authorization =
            HeaderValue::from_str(&format!("Bearer {}", config.management_root_key))
                .map_err(|_| UnkeyError::InvalidConfiguration)?;
        authorization.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, authorization);
        // reqwest's no-provider backend requires explicit process initialization.
        // Rustls installs once atomically; concurrent clients retain the provider
        // already installed by another client. SQLx/SMTP share the ring provider.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = Client::builder()
            .default_headers(headers)
            .connect_timeout(Duration::from_secs(2))
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .no_proxy()
            .https_only(endpoint.scheme() == "https")
            .build()
            .map_err(|_| UnkeyError::Unavailable)?;
        Ok(Self {
            client,
            endpoint,
            api_id: config.api_id.clone(),
            keyspace_id: config.keyspace_id.clone(),
            requests_per_minute: config.requests_per_minute,
        })
    }

    pub fn keyspace_id(&self) -> &str {
        &self.keyspace_id
    }

    #[cfg(test)]
    pub(crate) fn fixture(config: &UnkeyConfig, endpoint: url::Url) -> Result<Self, UnkeyError> {
        Self::build(config, endpoint, REQUEST_TIMEOUT)
    }

    pub async fn create_key(
        &self,
        owner: Uuid,
        input: &KeyCreate,
        now: DateTime<Utc>,
    ) -> Result<CreatedKey, UnkeyError> {
        input.validate()?;
        let expires_at = now
            .checked_add_signed(ChronoDuration::days(i64::from(input.expires_in_days)))
            .filter(|expires| expires.timestamp_millis() <= 4_102_444_800_000)
            .ok_or(UnkeyError::InvalidInput)?;
        let body = serde_json::json!({
            "apiId": self.api_id,
            "prefix": "tc",
            "name": input.name,
            "byteLength": 32,
            "externalId": owner.to_string(),
            "permissions": input.permissions,
            "expires": expires_at.timestamp_millis(),
            "ratelimits": [{"name":"requests", "limit":self.requests_per_minute, "duration":60000, "autoApply":true}],
            "enabled": true,
            "recoverable": false
        });
        // No retry, including protocol-level retry after a timeout/reset. Once a
        // request could have reached the provider, an unknown result is ambiguous.
        let response = self
            .client
            .post(self.endpoint.join("v2/keys.createKey").expect("fixed path"))
            .json(&body)
            .send()
            .await
            .map_err(|_| UnkeyError::AmbiguousCreate)?;
        let ambiguous = response.status().is_success() || response.status().is_server_error();
        if !response.status().is_success() {
            return Err(if ambiguous {
                UnkeyError::AmbiguousCreate
            } else {
                UnkeyError::Unavailable
            });
        }
        let bytes = bounded_response(response)
            .await
            .map_err(|_| UnkeyError::AmbiguousCreate)?;
        let wire: Envelope<KeyWire> =
            serde_json::from_slice(&bytes).map_err(|_| UnkeyError::AmbiguousCreate)?;
        if wire.error.is_some()
            || !identifier(&wire.data.key_id, "key_")
            || !(32..=512).contains(&wire.data.key.len())
            || !wire
                .data
                .key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err(UnkeyError::AmbiguousCreate);
        }
        Ok(CreatedKey {
            key_id: wire.data.key_id,
            secret: wire.data.key,
            expires_at,
        })
    }

    pub async fn delete_key(&self, key_id: &str) -> Result<(), UnkeyError> {
        if !identifier(key_id, "key_") {
            return Err(UnkeyError::InvalidInput);
        }
        let response = self
            .client
            .post(self.endpoint.join("v2/keys.deleteKey").expect("fixed path"))
            .json(&serde_json::json!({"keyId":key_id, "permanent":false}))
            .send()
            .await
            .map_err(|_| UnkeyError::Unavailable)?;
        let status = response.status();
        let bytes = bounded_response(response).await?;
        if status == StatusCode::OK {
            let envelope: Envelope<serde_json::Map<String, serde_json::Value>> =
                serde_json::from_slice(&bytes).map_err(|_| UnkeyError::Unavailable)?;
            if envelope.error.is_none() && envelope.data.is_empty() {
                return Ok(());
            }
        } else if status == StatusCode::NOT_FOUND {
            // A generic 404 may mean missing keyspace/root permissions; only an
            // exact key-not-found type proves an already deleted key.
            let value: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|_| UnkeyError::Unavailable)?;
            if matches!(
                value["error"]["type"].as_str(),
                Some(
                    "https://unkey.com/docs/errors/unkey/data/key_not_found"
                        | "https://www.unkey.com/docs/errors/unkey/data/key_not_found"
                )
            ) {
                return Ok(());
            }
        }
        Err(UnkeyError::Unavailable)
    }
}

fn identifier(value: &str, prefix: &str) -> bool {
    value.starts_with(prefix)
        && value.len() > prefix.len()
        && value.len() <= 255
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

async fn bounded_response(
    mut response: reqwest::Response,
) -> Result<Zeroizing<Vec<u8>>, UnkeyError> {
    if response
        .content_length()
        .is_some_and(|length| length > RESPONSE_BYTES as u64)
    {
        return Err(UnkeyError::Unavailable);
    }
    let mut bytes = Zeroizing::new(Vec::new());
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| UnkeyError::Unavailable)?
    {
        if bytes.len() + chunk.len() > RESPONSE_BYTES {
            return Err(UnkeyError::Unavailable);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[derive(Deserialize)]
struct Envelope<T> {
    data: T,
    #[serde(default)]
    error: Option<serde::de::IgnoredAny>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct KeyWire {
    key_id: String,
    key: Zeroizing<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        extract::{Json, State},
        http::HeaderMap,
        response::IntoResponse,
        routing::post,
    };
    use std::sync::{Arc, Mutex};

    const ROOT: &str = "fixture_management_credential_not_a_real_root_key";
    const KEY: &str = "tc_fixture_one_time_secret_not_a_real_provider_key_1234";
    type Recorded = Arc<Mutex<Vec<(HeaderMap, serde_json::Value)>>>;

    fn config() -> UnkeyConfig {
        UnkeyConfig {
            management_root_key: ROOT.into(),
            api_id: "api_fixture".into(),
            keyspace_id: "ks_fixture".into(),
            requests_per_minute: 60,
        }
    }
    fn input() -> KeyCreate {
        serde_json::from_value(serde_json::json!({"name":"Fixture automation"})).unwrap()
    }

    struct Fixture {
        requests: Recorded,
        task: tokio::task::JoinHandle<()>,
        client: Management,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    async fn fixture(status: StatusCode, body: String, delay: Duration) -> Fixture {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let handler = move |State(requests): State<Recorded>,
                            headers: HeaderMap,
                            Json(value): Json<serde_json::Value>| {
            let body = body.clone();
            async move {
                requests.lock().unwrap().push((headers, value));
                tokio::time::sleep(delay).await;
                (status, body).into_response()
            }
        };
        let router = Router::new()
            .route("/v2/keys.createKey", post(handler.clone()))
            .route("/v2/keys.deleteKey", post(handler))
            .with_state(requests.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint =
            url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        // Only unit tests can select this HTTP fixture; the runtime constructor
        // always uses the fixed HTTPS API Management origin.
        let client = Management::build(&config(), endpoint, Duration::from_millis(100)).unwrap();
        Fixture {
            requests,
            task,
            client,
        }
    }

    #[test]
    fn policy_defaults_are_read_only_expiring_and_cannot_grant_foreign_or_unknown_fields() {
        let defaults = input();
        assert_eq!(defaults.expires_in_days, 90);
        assert_eq!(defaults.permissions, read_only());
        for invalid in [
            serde_json::json!({"name":"Fixture","ownerId":Uuid::new_v4()}),
            serde_json::json!({"name":"Fixture","externalId":"another-owner"}),
            serde_json::json!({"name":"Fixture","recoverable":true}),
            serde_json::json!({"name":"Fixture","permissions":["admin"]}),
        ] {
            assert!(serde_json::from_value::<KeyCreate>(invalid).is_err());
        }
        for days in [0, 366] {
            let mut value = input();
            value.expires_in_days = days;
            assert_eq!(value.validate(), Err(UnkeyError::InvalidInput));
        }
        for name in [
            "".to_owned(),
            " ".into(),
            "x".repeat(101),
            "line\nbreak".into(),
        ] {
            let mut value = input();
            value.name = name;
            assert_eq!(value.validate(), Err(UnkeyError::InvalidInput));
        }
        for permissions in [vec![], vec![Permission::PostsRead, Permission::PostsRead]] {
            let mut value = input();
            value.permissions = permissions;
            assert_eq!(value.validate(), Err(UnkeyError::InvalidInput));
        }
        for days in [1, 90, 365] {
            let mut value = input();
            value.expires_in_days = days;
            assert!(value.validate().is_ok());
        }
    }

    #[tokio::test]
    async fn creation_contract_uses_api_id_owner_permissions_expiry_and_automatic_limits() {
        assert!(
            Management::new(&config()).is_ok(),
            "real TLS client must initialize with the explicit ring provider"
        );
        let fixture = fixture(StatusCode::OK,
            serde_json::json!({"meta":{"requestId":"fixture"},"data":{"keyId":"key_fixture","key":KEY}}).to_string(),
            Duration::ZERO).await;
        let owner = Uuid::new_v4();
        let now = Utc::now();
        let grant = fixture
            .client
            .create_key(owner, &input(), now)
            .await
            .unwrap();
        assert_eq!(grant.key_id, "key_fixture");
        assert!(grant.expose_secret() == KEY);
        assert_ne!(grant.masked_hint(), KEY);
        assert!(grant.masked_hint().len() <= 32);
        assert_eq!(fixture.client.keyspace_id(), "ks_fixture");
        let requests = fixture.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].0[AUTHORIZATION], format!("Bearer {ROOT}"));
        let request = &requests[0].1;
        assert_eq!(request["apiId"], "api_fixture");
        assert!(request.get("keyspaceId").is_none());
        assert_eq!(request["externalId"], owner.to_string());
        assert_eq!(request["byteLength"], 32);
        assert_eq!(request["recoverable"], false);
        assert_eq!(request["enabled"], true);
        assert_eq!(
            request["permissions"],
            serde_json::to_value(read_only()).unwrap()
        );
        assert_eq!(
            request["expires"],
            (now + ChronoDuration::days(90)).timestamp_millis()
        );
        assert_eq!(
            request["ratelimits"],
            serde_json::json!([{"name":"requests","limit":60,"duration":60000,"autoApply":true}])
        );
        assert!(request.get("roles").is_none());
        assert!(request.get("meta").is_none());
    }

    #[tokio::test]
    async fn invalid_input_never_calls_the_provider_and_configuration_errors_do_not_echo_secrets() {
        let fixture = fixture(StatusCode::OK, "{}".into(), Duration::ZERO).await;
        let mut request = input();
        request.expires_in_days = 366;
        assert!(matches!(
            fixture
                .client
                .create_key(Uuid::new_v4(), &request, Utc::now())
                .await,
            Err(UnkeyError::InvalidInput)
        ));
        assert!(fixture.requests.lock().unwrap().is_empty());
        for (api, keyspace) in [
            ("ks_wrong", "ks_fixture"),
            ("api_fixture", "api_wrong"),
            ("api_", "ks_fixture"),
        ] {
            let mut configuration = config();
            configuration.api_id = api.into();
            configuration.keyspace_id = keyspace.into();
            let error = Management::new(&configuration).err().unwrap();
            assert!(!format!("{error:?} {error}").contains(ROOT));
        }
    }

    #[tokio::test]
    async fn ambiguous_timeouts_server_errors_and_malformed_successes_are_never_retried_or_returned()
     {
        for (status, body, delay, expected) in [
            (StatusCode::OK, serde_json::json!({"data":{"keyId":"key_fixture","key":KEY}}).to_string(), Duration::from_millis(300), UnkeyError::AmbiguousCreate),
            (StatusCode::INTERNAL_SERVER_ERROR, format!("private-provider-detail-{KEY}"), Duration::ZERO, UnkeyError::AmbiguousCreate),
            (StatusCode::OK, "malformed".into(), Duration::ZERO, UnkeyError::AmbiguousCreate),
            (StatusCode::OK, "x".repeat(RESPONSE_BYTES + 1), Duration::ZERO, UnkeyError::AmbiguousCreate),
            (StatusCode::OK, serde_json::json!({"data":{"keyId":"foreign","key":KEY}}).to_string(), Duration::ZERO, UnkeyError::AmbiguousCreate),
            (StatusCode::OK, serde_json::json!({"data":{"keyId":"key_fixture","key":KEY},"error":{"detail":"failure"}}).to_string(), Duration::ZERO, UnkeyError::AmbiguousCreate),
            (StatusCode::UNAUTHORIZED, format!("private-provider-detail-{ROOT}"), Duration::ZERO, UnkeyError::Unavailable),
            (StatusCode::TOO_MANY_REQUESTS, "{}".into(), Duration::ZERO, UnkeyError::Unavailable),
        ] {
            let fixture = fixture(status, body, delay).await;
            let error = fixture.client.create_key(Uuid::new_v4(), &input(), Utc::now()).await.err().unwrap();
            assert_eq!(error, expected);
            assert_eq!(fixture.requests.lock().unwrap().len(), 1);
            assert!(!format!("{error:?} {error}").contains(KEY));
            assert!(!format!("{error:?} {error}").contains(ROOT));
        }
    }

    #[tokio::test]
    async fn deletion_is_soft_and_only_exact_missing_key_errors_are_idempotent_success() {
        for (status, body, expected) in [
            (
                StatusCode::OK,
                serde_json::json!({"meta":{"requestId":"fixture"},"data":{}}),
                Ok(()),
            ),
            (
                StatusCode::NOT_FOUND,
                serde_json::json!({"error":{"type":"https://unkey.com/docs/errors/unkey/data/key_not_found"}}),
                Ok(()),
            ),
            (
                StatusCode::NOT_FOUND,
                serde_json::json!({"error":{"type":"https://unkey.com/docs/errors/unkey/data/api_not_found"}}),
                Err(UnkeyError::Unavailable),
            ),
            (
                StatusCode::FORBIDDEN,
                serde_json::json!({"error":{"detail":ROOT}}),
                Err(UnkeyError::Unavailable),
            ),
            (
                StatusCode::OK,
                serde_json::json!({"data":{},"error":{"detail":KEY}}),
                Err(UnkeyError::Unavailable),
            ),
        ] {
            let fixture = fixture(status, body.to_string(), Duration::ZERO).await;
            assert_eq!(fixture.client.delete_key("key_fixture").await, expected);
            let requests = fixture.requests.lock().unwrap();
            assert_eq!(requests.len(), 1);
            assert_eq!(
                requests[0].1,
                serde_json::json!({"keyId":"key_fixture","permanent":false})
            );
        }
    }
}
