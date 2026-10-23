use std::{collections::HashMap, error::Error, fmt, net::SocketAddr};
use url::Url;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Environment {
    Development,
    Production,
}

// Intentionally not Debug: the configuration contains credentials.
pub struct Config {
    pub environment: Environment,
    pub browser_bind: SocketAddr,
    pub gateway_bind: SocketAddr,
    pub app_origin: Url,
    pub cookie_secure: bool,
    pub database_url: String,
    pub csrf_secret: String,
    pub rate_limit_secret: String,
    pub auth_limits: AuthLimits,
    pub social_writes_per_minute: u32,
    pub smtp: SmtpConfig,
    pub unkey: Option<UnkeyConfig>,
}

pub struct AuthLimits {
    pub login_account_ip: u32,
    pub login_ip: u32,
    pub registration_ip: u32,
    pub recovery_account_ip: u32,
    pub recovery_ip: u32,
    pub reauthentication_account_ip: u32,
    pub reauthentication_ip: u32,
}

pub struct SmtpConfig {
    pub from_address: String,
    pub host: String,
    pub port: u16,
    pub tls: bool,
    pub username: Option<String>,
    pub password: Option<String>,
}

pub struct UnkeyConfig {
    pub management_root_key: String,
    pub api_id: String,
    pub keyspace_id: String,
    pub requests_per_minute: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ConfigError(&'static str);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl Error for ConfigError {}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_values(&std::env::vars().collect())
    }

    pub fn from_values(values: &HashMap<String, String>) -> Result<Self, ConfigError> {
        let get = |key: &str| values.get(key).filter(|s| !s.is_empty()).cloned();
        let environment = match get("APP_ENV").as_deref().unwrap_or("development") {
            "development" => Environment::Development,
            "production" => Environment::Production,
            _ => return Err(ConfigError("APP_ENV must be development or production")),
        };
        let production = environment == Environment::Production;
        let browser_bind: SocketAddr = get("BROWSER_BIND")
            .unwrap_or_else(|| "127.0.0.1:8080".into())
            .parse()
            .map_err(|_| ConfigError("BROWSER_BIND must be an IP address and port"))?;
        let gateway_bind: SocketAddr = get("GATEWAY_BIND")
            .unwrap_or_else(|| "127.0.0.1:8081".into())
            .parse()
            .map_err(|_| ConfigError("GATEWAY_BIND must be an IP address and port"))?;
        if browser_bind == gateway_bind {
            return Err(ConfigError(
                "Browser and gateway listeners must be distinct",
            ));
        }
        let app_origin =
            Url::parse(&get("APP_ORIGIN").unwrap_or_else(|| "http://localhost:3000".into()))
                .map_err(|_| ConfigError("APP_ORIGIN must be a valid HTTP origin"))?;
        if !matches!(app_origin.scheme(), "http" | "https")
            || app_origin.host_str().is_none()
            || !app_origin.username().is_empty()
            || app_origin.password().is_some()
            || app_origin.path() != "/"
            || app_origin.query().is_some()
            || app_origin.fragment().is_some()
        {
            return Err(ConfigError(
                "APP_ORIGIN must contain only scheme, host, and port",
            ));
        }
        let cookie_secure = boolean(values, "COOKIE_SECURE", production)?;
        if production && (!cookie_secure || app_origin.scheme() != "https") {
            return Err(ConfigError(
                "Production requires HTTPS and Secure session cookies",
            ));
        }
        if !production && !cookie_secure {
            let host = app_origin.host_str().unwrap_or_default();
            if host != "localhost" && host != "127.0.0.1" && host != "[::1]" {
                return Err(ConfigError(
                    "Insecure development cookies require a loopback origin",
                ));
            }
        }
        let database_url = get("DATABASE_URL").ok_or(ConfigError("DATABASE_URL is required"))?;
        let database = Url::parse(&database_url)
            .map_err(|_| ConfigError("DATABASE_URL must be a valid PostgreSQL URL"))?;
        if !matches!(database.scheme(), "postgres" | "postgresql") || database.host_str().is_none()
        {
            return Err(ConfigError("DATABASE_URL must be a valid PostgreSQL URL"));
        }
        if production {
            if !browser_bind.ip().is_loopback()
                || !gateway_bind.ip().is_loopback()
                || browser_bind.port() == 0
                || gateway_bind.port() == 0
            {
                return Err(ConfigError(
                    "Production Rust listeners must use nonzero loopback ports",
                ));
            }
            let parameters: Vec<_> = database.query_pairs().collect();
            if database.username().is_empty()
                || database
                    .password()
                    .is_none_or(|password| password.trim().is_empty())
                || database.path().trim_matches('/').is_empty()
                || local_host(database.host_str().unwrap_or_default())
                || parameters
                    .iter()
                    .filter(|(key, _)| key == "sslmode")
                    .count()
                    != 1
                || !parameters
                    .iter()
                    .any(|(key, value)| key == "sslmode" && value == "verify-full")
                || parameters.iter().any(|(key, _)| {
                    !matches!(
                        key.as_ref(),
                        "sslmode" | "sslrootcert" | "sslcert" | "sslkey" | "application_name"
                    )
                })
            {
                return Err(ConfigError(
                    "Production DATABASE_URL requires an external authenticated database, sslmode=verify-full, and only canonical TLS/application_name parameters",
                ));
            }
        }
        let csrf_secret = required_secret(
            get("CSRF_SECRET"),
            "CSRF_SECRET is required and must contain at least 32 bytes in every environment",
        )?;
        let rate_limit_secret = required_secret(
            get("RATE_LIMIT_SECRET"),
            "RATE_LIMIT_SECRET is required and must contain at least 32 bytes in every environment",
        )?;
        if csrf_secret == rate_limit_secret {
            return Err(ConfigError(
                "CSRF_SECRET and RATE_LIMIT_SECRET must be distinct",
            ));
        }
        let auth_limits = AuthLimits {
            login_account_ip: positive_limit(values, "AUTH_LOGIN_ACCOUNT_IP_LIMIT", 5)?,
            login_ip: positive_limit(values, "AUTH_LOGIN_IP_LIMIT", 50)?,
            registration_ip: positive_limit(values, "AUTH_REGISTRATION_IP_LIMIT", 5)?,
            recovery_account_ip: positive_limit(values, "AUTH_RECOVERY_ACCOUNT_IP_LIMIT", 3)?,
            recovery_ip: positive_limit(values, "AUTH_RECOVERY_IP_LIMIT", 20)?,
            reauthentication_account_ip: positive_limit(values, "AUTH_REAUTH_ACCOUNT_IP_LIMIT", 5)?,
            reauthentication_ip: positive_limit(values, "AUTH_REAUTH_IP_LIMIT", 50)?,
        };
        let social_writes_per_minute = positive_limit(values, "SOCIAL_WRITES_PER_MINUTE", 30)?;
        let smtp = SmtpConfig {
            from_address: match get("EMAIL_FROM") {
                Some(value) => value,
                None if production => {
                    return Err(ConfigError("EMAIL_FROM is required in production"));
                }
                None => "noreply@example.test".into(),
            },
            host: get("SMTP_HOST").unwrap_or_else(|| "mail".into()),
            port: get("SMTP_PORT")
                .unwrap_or_else(|| "1025".into())
                .parse()
                .ok()
                .filter(|port| *port != 0)
                .ok_or(ConfigError("SMTP_PORT must be a nonzero port number"))?,
            tls: boolean(values, "SMTP_TLS", false)?,
            username: get("SMTP_USERNAME"),
            password: get("SMTP_PASSWORD"),
        };
        let sender = smtp
            .from_address
            .parse::<email_address::EmailAddress>()
            .map_err(|_| ConfigError("EMAIL_FROM must be a single email address"))?;
        if !sender.display_part().is_empty() {
            return Err(ConfigError("EMAIL_FROM must be a single email address"));
        }
        let sender_domain = sender.domain().to_ascii_lowercase();
        if production && (sender_domain.ends_with(".test") || sender_domain == "localhost") {
            return Err(ConfigError(
                "Production EMAIL_FROM must not use a development domain",
            ));
        }
        if smtp.username.is_some() != smtp.password.is_some() {
            return Err(ConfigError(
                "SMTP_USERNAME and SMTP_PASSWORD must be supplied together",
            ));
        }
        if production
            && (!smtp.tls
                || local_host(&smtp.host)
                || smtp.host.chars().any(char::is_whitespace)
                || matches!(
                    smtp.host.to_lowercase().as_str(),
                    "mail" | "mailpit" | "localhost" | "127.0.0.1" | "::1"
                )
                || smtp
                    .username
                    .as_ref()
                    .is_none_or(|value| value.trim().is_empty())
                || smtp
                    .password
                    .as_ref()
                    .is_none_or(|value| value.trim().is_empty()))
        {
            return Err(ConfigError(
                "Production requires external SMTP with TLS and authentication",
            ));
        }
        let unkey = match (
            get("UNKEY_MANAGEMENT_ROOT_KEY"),
            get("UNKEY_API_ID"),
            get("UNKEY_KEYSPACE_ID"),
        ) {
            (None, None, None) => None,
            (Some(management_root_key), Some(api_id), Some(keyspace_id))
                if api_id.starts_with("api_") && keyspace_id.starts_with("ks_") =>
            {
                Some(UnkeyConfig {
                    management_root_key,
                    api_id,
                    keyspace_id,
                    requests_per_minute: positive_limit(values, "UNKEY_REQUESTS_PER_MINUTE", 60)?,
                })
            }
            _ => {
                return Err(ConfigError(
                    "Unkey requires a management credential, api_ API ID, and ks_ keyspace ID together",
                ));
            }
        };
        if get("WEB_RUNTIME_MODE").as_deref() == Some("compute") {
            if !production || get("HOST").as_deref() != Some("0.0.0.0") {
                return Err(ConfigError(
                    "Compute requires APP_ENV=production and HOST=0.0.0.0",
                ));
            }
            let port = get("PORT")
                .filter(|port| port.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|port| port.parse::<u16>().ok())
                .filter(|port| *port > 0)
                .ok_or(ConfigError("Compute PORT must be 1–65535"))?;
            if port == browser_bind.port() || port == gateway_bind.port() {
                return Err(ConfigError(
                    "Compute PORT must not collide with either Rust listener",
                ));
            }
            for (name, bind) in [
                ("API_BASE_URL", browser_bind),
                ("GATEWAY_API_BASE_URL", gateway_bind),
            ] {
                let origin = get(name).and_then(|value| Url::parse(&value).ok());
                let expected = Url::parse(&format!("http://{bind}")).expect("socket origin");
                if origin.as_ref() != Some(&expected) {
                    return Err(ConfigError(
                        "Compute upstream origins must exactly match the loopback Rust bindings",
                    ));
                }
            }
            if unkey.is_none() {
                return Err(ConfigError(
                    "Compute requires environment-specific Unkey management configuration",
                ));
            }
            if values.keys().any(|key| {
                matches!(
                    key.as_str(),
                    "NITRO_PORT" | "NITRO_HOST" | "NITRO_UNIX_SOCKET"
                )
            }) {
                return Err(ConfigError(
                    "Compute must use PORT/HOST, not Nitro port/host/socket overrides",
                ));
            }
        }
        Ok(Self {
            environment,
            browser_bind,
            gateway_bind,
            app_origin,
            cookie_secure,
            database_url,
            csrf_secret,
            rate_limit_secret,
            auth_limits,
            social_writes_per_minute,
            smtp,
            unkey,
        })
    }
}

fn local_host(host: &str) -> bool {
    let host = host.trim_matches(['[', ']']).to_ascii_lowercase();
    matches!(
        host.as_str(),
        "localhost" | "db" | "postgres" | "postgresql"
    ) || host.ends_with(".localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback() || ip.is_unspecified())
}

fn required_secret(value: Option<String>, message: &'static str) -> Result<String, ConfigError> {
    value
        .filter(|secret| secret_is_valid(secret))
        .ok_or(ConfigError(message))
}

pub(crate) fn secret_is_valid(secret: &str) -> bool {
    secret.len() >= 32 && !secret.trim().is_empty()
}

fn positive_limit(
    values: &HashMap<String, String>,
    key: &str,
    default: u32,
) -> Result<u32, ConfigError> {
    match values.get(key).map(String::as_str) {
        None | Some("") => Ok(default),
        Some(value) => value
            .parse()
            .ok()
            .filter(|limit| *limit > 0)
            .ok_or(ConfigError("Attempt limits must be positive integers")),
    }
}

fn boolean(
    values: &HashMap<String, String>,
    key: &str,
    default: bool,
) -> Result<bool, ConfigError> {
    match values.get(key).map(String::as_str) {
        None | Some("") => Ok(default),
        Some("true") => Ok(true),
        Some("false") => Ok(false),
        _ => Err(ConfigError(
            "Boolean settings must be exactly true or false",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn development() -> HashMap<String, String> {
        HashMap::from([
            ("DATABASE_URL".into(), "postgres://localhost/test".into()),
            (
                "CSRF_SECRET".into(),
                "test-fixture-csrf-value-not-a-real-secret".into(),
            ),
            (
                "RATE_LIMIT_SECRET".into(),
                "test-fixture-rate-limit-not-a-real-secret".into(),
            ),
        ])
    }

    fn production() -> HashMap<String, String> {
        let mut values = development();
        for (key, value) in [
            ("APP_ENV", "production"),
            ("APP_ORIGIN", "https://social.example.com"),
            ("EMAIL_FROM", "noreply@example.com"),
            ("SMTP_HOST", "smtp.example.com"),
            ("SMTP_PORT", "587"),
            ("SMTP_TLS", "true"),
            ("SMTP_USERNAME", "test-only"),
            ("SMTP_PASSWORD", "test-fixture-not-a-real-credential"),
            (
                "DATABASE_URL",
                "postgres://fixture:fixture@database.example.com/social?sslmode=verify-full",
            ),
        ] {
            values.insert(key.into(), value.into());
        }
        values
    }

    fn reject(values: &HashMap<String, String>, expected: &str) {
        let error = Config::from_values(values)
            .err()
            .expect("must reject config");
        assert!(error.to_string().contains(expected), "{error}");
    }

    #[test]
    fn development_does_not_require_unkey() {
        let config = Config::from_values(&development()).unwrap();
        assert!(config.unkey.is_none());
        assert!(!config.cookie_secure);
        assert!(config.browser_bind.ip().is_loopback());
        assert!(config.gateway_bind.ip().is_loopback());
    }

    #[test]
    fn auth_attempt_limits_are_configurable_but_cannot_be_disabled() {
        for key in [
            "AUTH_LOGIN_ACCOUNT_IP_LIMIT",
            "AUTH_LOGIN_IP_LIMIT",
            "AUTH_REGISTRATION_IP_LIMIT",
            "AUTH_RECOVERY_ACCOUNT_IP_LIMIT",
            "AUTH_RECOVERY_IP_LIMIT",
            "AUTH_REAUTH_ACCOUNT_IP_LIMIT",
            "AUTH_REAUTH_IP_LIMIT",
            "SOCIAL_WRITES_PER_MINUTE",
        ] {
            for invalid in ["0", "-1", "invalid-private-fixture", "4294967296"] {
                let mut values = development();
                values.insert(key.into(), invalid.into());
                reject(&values, "positive integers");
            }
        }
        let mut values = development();
        values.insert("AUTH_LOGIN_IP_LIMIT".into(), "12".into());
        let config = Config::from_values(&values).unwrap();
        assert_eq!(config.auth_limits.login_ip, 12);
        assert_eq!(config.auth_limits.login_account_ip, 5);
    }

    #[test]
    fn secure_production_is_accepted() {
        assert!(Config::from_values(&production()).is_ok());
    }

    #[test]
    fn production_requires_verified_database_tls_without_parameter_bypasses() {
        for database in [
            "postgres://fixture:fixture@database.example.com/social",
            "postgres://fixture:fixture@database.example.com/social?sslmode=require",
            "postgres://fixture:fixture@database.example.com/social?sslmode=verify-ca",
            "postgres://fixture:fixture@database.example.com/social?sslmode=verify-full&sslmode=disable",
            "postgres://fixture:fixture@database.example.com/social?sslmode=verify-full&ssl-mode=disable",
            "postgres://fixture@database.example.com/social?sslmode=verify-full",
            "postgres://fixture:fixture@localhost/social?sslmode=verify-full",
            "postgres://fixture:fixture@database.example.com/social?sslmode=verify-full&host=localhost",
        ] {
            let mut values = production();
            values.insert("DATABASE_URL".into(), database.into());
            reject(&values, "DATABASE_URL");
        }
    }

    #[test]
    fn compute_validates_platform_ports_loopback_origins_and_required_configuration() {
        let mut values = production();
        for (key, value) in [
            ("WEB_RUNTIME_MODE", "compute"),
            ("HOST", "0.0.0.0"),
            ("PORT", "8080"),
            ("BROWSER_BIND", "127.0.0.1:8081"),
            ("GATEWAY_BIND", "127.0.0.1:8082"),
            ("API_BASE_URL", "http://127.0.0.1:8081"),
            ("GATEWAY_API_BASE_URL", "http://127.0.0.1:8082"),
            ("UNKEY_MANAGEMENT_ROOT_KEY", "fixture-not-a-real-key"),
            ("UNKEY_API_ID", "api_fixture"),
            ("UNKEY_KEYSPACE_ID", "ks_fixture"),
        ] {
            values.insert(key.into(), value.into());
        }
        assert!(Config::from_values(&values).is_ok());
        for (key, value, expected) in [
            ("PORT", "", "PORT"),
            ("PORT", "0", "PORT"),
            ("PORT", "65536", "PORT"),
            ("PORT", "8081", "collide"),
            ("HOST", "127.0.0.1", "HOST"),
            ("BROWSER_BIND", "0.0.0.0:8081", "loopback"),
            ("API_BASE_URL", "https://public.example.com", "upstream"),
            ("GATEWAY_API_BASE_URL", "http://127.0.0.1:8081", "upstream"),
            ("NITRO_PORT", "9999", "Nitro"),
        ] {
            let mut invalid = values.clone();
            invalid.insert(key.into(), value.into());
            reject(&invalid, expected);
        }
    }

    #[test]
    fn sender_and_smtp_credentials_are_validated_without_echoing_values() {
        for baseline in [development(), production()] {
            let mut values = baseline.clone();
            values.insert("EMAIL_FROM".into(), "private-invalid-sender-fixture".into());
            let error = Config::from_values(&values)
                .err()
                .expect("must reject sender");
            assert!(error.to_string().contains("EMAIL_FROM"));
            assert!(!error.to_string().contains("private-invalid-sender-fixture"));
            values.insert("EMAIL_FROM".into(), "Name <fixture@example.com>".into());
            reject(&values, "EMAIL_FROM");
        }
        let mut values = production();
        values.remove("EMAIL_FROM");
        reject(&values, "EMAIL_FROM");
        values.insert("EMAIL_FROM".into(), "fixture@EXAMPLE.TEST".into());
        reject(&values, "development domain");
        let mut values = development();
        values.insert("SMTP_USERNAME".into(), "test-only".into());
        reject(&values, "supplied together");
    }

    #[test]
    fn unsafe_production_is_rejected() {
        for (key, value, expected) in [
            ("COOKIE_SECURE", "false", "HTTPS"),
            ("APP_ORIGIN", "http://social.example.com", "HTTPS"),
            ("SMTP_TLS", "false", "SMTP"),
            ("SMTP_HOST", "mail", "SMTP"),
            ("SMTP_PASSWORD", "", "SMTP"),
        ] {
            let mut values = production();
            values.insert(key.into(), value.into());
            reject(&values, expected);
        }
    }

    #[test]
    fn insecure_development_is_loopback_only() {
        let mut values = development();
        values.insert("APP_ORIGIN".into(), "http://social.example.com".into());
        reject(&values, "loopback");
    }

    #[test]
    fn invalid_configuration_errors_do_not_echo_values() {
        for (key, value) in [
            ("APP_ENV", "bad-secret-environment"),
            ("BROWSER_BIND", "bad-secret-address"),
            ("DATABASE_URL", "bad-secret-database-password"),
            ("APP_ORIGIN", "https://user:bad-secret-password@example.com"),
            ("SMTP_PORT", "bad-secret-port"),
        ] {
            let mut values = development();
            values.insert(key.into(), value.into());
            let error = Config::from_values(&values).err().expect("must reject");
            assert!(!error.to_string().contains("bad-secret"));
        }
    }

    #[test]
    fn partial_unkey_configuration_is_rejected() {
        let mut values = development();
        values.insert("UNKEY_API_ID".into(), "api_test".into());
        reject(&values, "Unkey");
    }

    #[test]
    fn credential_bearing_or_non_origin_urls_are_rejected() {
        for origin in [
            "https://example.com/path",
            "https://example.com?token=secret",
            "https://example.com#secret",
            "https://user:password@example.com",
            "ftp://example.com",
        ] {
            let mut values = development();
            values.insert("APP_ORIGIN".into(), origin.into());
            reject(&values, "APP_ORIGIN");
        }
    }

    #[test]
    fn listener_collision_is_rejected() {
        let mut values = development();
        values.insert("GATEWAY_BIND".into(), "127.0.0.1:8080".into());
        reject(&values, "distinct");
    }

    #[test]
    fn secrets_are_required_and_validated_in_every_environment() {
        for baseline in [development(), production()] {
            for key in ["CSRF_SECRET", "RATE_LIMIT_SECRET"] {
                for invalid in [
                    None,
                    Some(""),
                    Some("short-test-fixture"),
                    Some("                                "),
                ] {
                    let mut values = baseline.clone();
                    match invalid {
                        Some(value) => {
                            values.insert(key.into(), value.into());
                        }
                        None => {
                            values.remove(key);
                        }
                    }
                    let error = Config::from_values(&values)
                        .err()
                        .expect("must reject secret");
                    assert!(error.to_string().contains(key));
                    assert!(error.to_string().contains("32 bytes"));
                    if let Some(value) = invalid.filter(|value| !value.trim().is_empty()) {
                        assert!(!error.to_string().contains(value));
                    }
                }
            }
        }
    }

    #[test]
    fn secret_byte_length_boundary_is_enforced() {
        for baseline in [development(), production()] {
            for key in ["CSRF_SECRET", "RATE_LIMIT_SECRET"] {
                let mut values = baseline.clone();
                values.insert(key.into(), "x".repeat(31));
                reject(&values, key);
                values.insert(key.into(), "x".repeat(32));
                assert!(Config::from_values(&values).is_ok());
            }
        }
    }

    #[test]
    fn reused_secrets_are_rejected_in_every_environment() {
        for mut values in [development(), production()] {
            values.insert("RATE_LIMIT_SECRET".into(), values["CSRF_SECRET"].clone());
            reject(&values, "distinct");
        }
    }

    #[test]
    fn configured_secrets_are_preserved_without_fallback_or_normalization() {
        let values = development();
        let first = Config::from_values(&values).unwrap();
        let second = Config::from_values(&values).unwrap();
        assert_eq!(first.csrf_secret, values["CSRF_SECRET"]);
        assert_eq!(first.rate_limit_secret, values["RATE_LIMIT_SECRET"]);
        assert_eq!(first.csrf_secret, second.csrf_secret);
        assert_eq!(first.rate_limit_secret, second.rate_limit_secret);
    }
}
