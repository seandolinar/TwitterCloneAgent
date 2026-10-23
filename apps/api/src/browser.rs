//! Cookie-only account adapter. Identity and CSRF context are request-scoped.
use crate::{
    accounts::{
        AccountError, Accounts, ActiveSession, clear_session_cookie, normalize_email,
        session_cookie_name,
    },
    config::Config,
    http::ApiError,
    keys::Keys,
    rate_limit::{RateLimitError, RateLimiter, Rule},
    social::Actor,
    social_http::SocialContext,
};
use axum::{
    Extension, Json, Router,
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use cookie::{Cookie, SameSite, time::Duration};
use csrf::{CsrfProtection, HmacCsrfProtection};
use data_encoding::BASE64;
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
};

const CSRF_SECONDS: i64 = 60 * 60;

pub struct BrowserAuth {
    pub accounts: Accounts,
    pub keys: Keys,
    config: Arc<Config>,
    limiter: RateLimiter,
}

// Deliberately not Debug: this contains private session material.
pub struct BrowserContext {
    pub session: Option<ActiveSession>,
    raw_session: Option<String>,
    ip: IpAddr,
    email_enabled: bool,
}

impl BrowserContext {
    pub fn authenticated(&self) -> Result<&ActiveSession, ApiError> {
        self.session.as_ref().ok_or(ApiError {
            status: StatusCode::UNAUTHORIZED,
            code: "authentication_required",
            message: "Log in to continue.",
        })
    }

    pub fn verified(&self) -> Result<&ActiveSession, ApiError> {
        let session = self.authenticated()?;
        if self.email_enabled && !session.identity.verified {
            return Err(ApiError {
                status: StatusCode::FORBIDDEN,
                code: "verification_required",
                message: "Verify your email to continue.",
            });
        }
        Ok(session)
    }
}

impl BrowserAuth {
    pub async fn new(pool: sqlx::PgPool, config: Arc<Config>) -> Result<Self, AccountError> {
        Ok(Self {
            accounts: Accounts::new(pool.clone())
                .await?
                .with_email_enabled(config.email_enabled),
            keys: Keys::new(pool.clone(), &config).map_err(|_| AccountError::Unavailable)?,
            limiter: RateLimiter::new(pool, &config.rate_limit_secret)
                .map_err(|_| AccountError::Unavailable)?,
            config,
        })
    }

    /// Apply to all browser account/social/key routes, not health endpoints.
    pub fn protect(self: &Arc<Self>, routes: Router) -> Router {
        routes.layer(middleware::from_fn_with_state(self.clone(), security))
    }

    pub fn routes(self: &Arc<Self>) -> Router {
        self.protect(
            Router::new()
                .route("/browser/auth/csrf", get(bootstrap))
                .route("/browser/auth/settings", get(account_settings))
                .route("/browser/auth/session", get(session))
                .route("/browser/auth/register", post(register))
                .route("/browser/auth/login", post(login))
                .route("/browser/auth/logout", post(logout))
                .route("/browser/auth/verify-email", post(verify_email))
                .route("/browser/auth/resend-verification", post(resend))
                .route("/browser/auth/forgot-password", post(forgot_password))
                .route("/browser/auth/reset-password", post(reset_password))
                .route("/browser/auth/reauthenticate", post(reauthenticate))
                .route("/browser/auth/keys", get(list_keys).post(create_key))
                .route("/browser/auth/keys/{id}", axum::routing::delete(revoke_key))
                .with_state(self.clone()),
        )
    }

    fn csrf_name(&self) -> &'static str {
        if self.config.cookie_secure {
            "__Host-csrf"
        } else {
            "twitterclone_dev_csrf"
        }
    }

    fn csrf(&self, binding: Option<&[u8]>) -> HmacCsrfProtection {
        // Delegate pair generation, parsing, expiry, and constant-time MAC checks
        // to the maintained CSRF library. Derive a separate key for each session;
        // the anonymous pair itself provides a signed random pre-login context.
        let mut mac = Hmac::<Sha256>::new_from_slice(self.config.csrf_secret.as_bytes())
            .expect("HMAC accepts a key of any length");
        mac.update(b"twitterclone:csrf:v1\0");
        match binding {
            Some(digest) => {
                mac.update(b"session\0");
                mac.update(digest);
            }
            None => mac.update(b"anonymous\0"),
        }
        HmacCsrfProtection::from_key(mac.finalize().into_bytes().into())
    }

    fn issue_csrf(&self, binding: Option<&[u8]>) -> Result<(String, String), AccountError> {
        let (token, cookie) = self
            .csrf(binding)
            .generate_token_pair(None, CSRF_SECONDS)
            .map_err(|_| AccountError::Unavailable)?;
        let cookie = Cookie::build((self.csrf_name(), cookie.b64_string()))
            .http_only(true)
            .secure(self.config.cookie_secure)
            .same_site(SameSite::Lax)
            .path("/")
            .max_age(Duration::seconds(CSRF_SECONDS))
            .build()
            .to_string();
        Ok((token.b64_string(), cookie))
    }

    fn check_csrf(&self, headers: &HeaderMap, binding: Option<&[u8]>) -> Result<(), ApiError> {
        let token = single_header(headers, "x-csrf-token").ok_or_else(csrf_error)?;
        let cookie = cookie_value(headers, self.csrf_name())?.ok_or_else(csrf_error)?;
        let token = BASE64.decode(token.as_bytes()).map_err(|_| csrf_error())?;
        let cookie = BASE64.decode(cookie.as_bytes()).map_err(|_| csrf_error())?;
        let protection = self.csrf(binding);
        let token = protection.parse_token(&token).map_err(|_| csrf_error())?;
        let cookie = protection.parse_cookie(&cookie).map_err(|_| csrf_error())?;
        protection
            .verify_token_pair(&token, &cookie)
            .map_err(|_| csrf_error())
    }

    async fn throttle(
        &self,
        context: &BrowserContext,
        action: &str,
        identifier: &str,
        pair_limit: u32,
        ip_limit: u32,
        seconds: u32,
    ) -> Result<(), RateLimitError> {
        let ip = context.ip.to_string();
        self.limiter
            .check(
                &format!("{action}:ip"),
                ip.as_bytes(),
                Rule::new(ip_limit, seconds).expect("positive server-owned rule"),
            )
            .await?;
        // JSON tuple encoding prevents ambiguous concatenation of private values.
        let pair = serde_json::to_vec(&(identifier, ip)).expect("strings serialize");
        self.limiter
            .check(
                &format!("{action}:account-ip"),
                &pair,
                Rule::new(pair_limit, seconds).expect("positive server-owned rule"),
            )
            .await
    }
}

fn single_header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?.to_str().ok()?;
    if values.next().is_some() {
        return None;
    }
    Some(value)
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Result<Option<String>, ApiError> {
    let mut result = None;
    for header in headers.get_all(header::COOKIE) {
        for pair in header.to_str().map_err(|_| csrf_error())?.split(';') {
            let pair = pair.trim();
            if pair.is_empty() {
                continue;
            }
            let cookie = Cookie::parse(pair.to_owned()).map_err(|_| csrf_error())?;
            if cookie.name() == name && result.replace(cookie.value().to_owned()).is_some() {
                return Err(csrf_error());
            }
        }
    }
    Ok(result)
}

fn csrf_error() -> ApiError {
    ApiError {
        status: StatusCode::FORBIDDEN,
        code: "csrf_rejected",
        message: "Refresh the security token and try again from this site.",
    }
}

async fn security(
    State(auth): State<Arc<BrowserAuth>>,
    mut request: Request,
    next: Next,
) -> Response {
    let unsafe_method = !matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    );
    let origin = single_header(request.headers(), "origin");
    let expected = auth.config.app_origin.origin().ascii_serialization();
    if (unsafe_method && origin != Some(expected.as_str()))
        || (request.headers().contains_key("origin") && origin != Some(expected.as_str()))
    {
        return csrf_error().into_response();
    }
    let raw_session = match cookie_value(request.headers(), session_cookie_name(&auth.config)) {
        Ok(value) => value,
        Err(error) => return error.into_response(),
    };
    let session = match raw_session.as_deref() {
        Some(token) => match auth.accounts.resolve_session(token).await {
            Ok(session) => session,
            Err(error) => return error.into_response(),
        },
        None => None,
    };
    if unsafe_method
        && let Err(error) = auth.check_csrf(
            request.headers(),
            session.as_ref().map(ActiveSession::binding),
        )
    {
        return error.into_response();
    }
    // This header belongs to the private frontend-to-browser-listener protocol.
    // The public proxy must replace it using trusted peer/hosting context.
    // Never derive client identity from public Forwarded/X-Forwarded-For headers.
    let ip = if request.headers().contains_key("x-twitterclone-client-ip") {
        match single_header(request.headers(), "x-twitterclone-client-ip")
            .and_then(|v| v.parse().ok())
        {
            Some(ip) => ip,
            None => return csrf_error().into_response(),
        }
    } else {
        match request.extensions().get::<ConnectInfo<SocketAddr>>() {
            Some(peer) => peer.0.ip(),
            None => {
                return ApiError {
                    status: StatusCode::SERVICE_UNAVAILABLE,
                    code: "client_context_unavailable",
                    message: "Trusted client context is unavailable.",
                }
                .into_response();
            }
        }
    };
    request.extensions_mut().insert(SocialContext {
        actor: session.as_ref().map(Actor::from_session),
    });
    request.extensions_mut().insert(Arc::new(BrowserContext {
        session,
        raw_session,
        ip,
        email_enabled: auth.config.email_enabled,
    }));
    next.run(request).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CsrfResponse {
    csrf_token: String,
}

fn csrf_response(auth: &BrowserAuth, binding: Option<&[u8]>) -> Result<Response, AccountError> {
    let (csrf_token, cookie) = auth.issue_csrf(binding)?;
    let mut response = Json(CsrfResponse { csrf_token }).into_response();
    set_cookie(&mut response, cookie);
    Ok(response)
}

fn set_cookie(response: &mut Response, value: String) {
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(&value).expect("generated cookie is a valid header"),
    );
}

async fn bootstrap(
    State(auth): State<Arc<BrowserAuth>>,
    Extension(ctx): Extension<Arc<BrowserContext>>,
) -> Result<Response, AccountError> {
    csrf_response(&auth, ctx.session.as_ref().map(ActiveSession::binding))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionResponse {
    id: String,
    handle: String,
    verified: bool,
    recent_password_auth: bool,
    email_enabled: bool,
}

async fn session(
    Extension(ctx): Extension<Arc<BrowserContext>>,
) -> Result<Json<SessionResponse>, ApiError> {
    let session = ctx.authenticated()?;
    Ok(Json(SessionResponse {
        id: session.identity.id.to_string(),
        handle: session.identity.handle.clone(),
        verified: session.identity.verified,
        recent_password_auth: session.recent_password_auth,
        email_enabled: ctx.email_enabled,
    }))
}

async fn account_settings(State(auth): State<Arc<BrowserAuth>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({"emailEnabled": auth.config.email_enabled}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Register {
    email: String,
    handle: String,
    password: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Login {
    email: String,
    password: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Email {
    email: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Token {
    token: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Password {
    password: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResetPassword {
    token: String,
    password: String,
}

async fn register(
    State(auth): State<Arc<BrowserAuth>>,
    Extension(ctx): Extension<Arc<BrowserContext>>,
    Json(input): Json<Register>,
) -> Response {
    let email = input.email.trim().to_lowercase();
    let limit = auth.config.auth_limits.registration_ip;
    if let Err(response) = auth
        .throttle(&ctx, "register", &email, limit, limit, 3600)
        .await
    {
        return response.into_response();
    }
    match auth
        .accounts
        .register(&input.email, &input.handle, &input.password)
        .await
    {
        Ok(()) => StatusCode::ACCEPTED.into_response(),
        Err(error) => error.into_response(),
    }
}

async fn login(
    State(auth): State<Arc<BrowserAuth>>,
    Extension(ctx): Extension<Arc<BrowserContext>>,
    Json(input): Json<Login>,
) -> Response {
    let email = input.email.trim().to_lowercase();
    let limits = &auth.config.auth_limits;
    if let Err(response) = auth
        .throttle(
            &ctx,
            "login",
            &email,
            limits.login_account_ip,
            limits.login_ip,
            900,
        )
        .await
    {
        return response.into_response();
    }
    match auth
        .accounts
        .login(&input.email, &input.password, ctx.raw_session.as_deref())
        .await
    {
        Ok(grant) => match csrf_response(&auth, Some(&grant.token.digest())) {
            Ok(mut response) => {
                set_cookie(&mut response, grant.cookie(&auth.config));
                response
            }
            Err(error) => error.into_response(),
        },
        Err(error) => error.into_response(),
    }
}

async fn logout(
    State(auth): State<Arc<BrowserAuth>>,
    Extension(ctx): Extension<Arc<BrowserContext>>,
) -> Response {
    if let Some(token) = &ctx.raw_session
        && let Err(error) = auth.accounts.logout(token).await
    {
        return error.into_response();
    }
    match csrf_response(&auth, None) {
        Ok(mut response) => {
            set_cookie(&mut response, clear_session_cookie(&auth.config));
            response
        }
        Err(error) => error.into_response(),
    }
}

async fn verify_email(
    State(auth): State<Arc<BrowserAuth>>,
    Extension(ctx): Extension<Arc<BrowserContext>>,
    Json(input): Json<Token>,
) -> Response {
    let limits = &auth.config.auth_limits;
    if let Err(response) = auth
        .throttle(
            &ctx,
            "verify",
            &input.token,
            limits.recovery_account_ip,
            limits.recovery_ip,
            900,
        )
        .await
    {
        return response.into_response();
    }
    match auth.accounts.verify_email(&input.token).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => error.into_response(),
    }
}

async fn resend(
    State(auth): State<Arc<BrowserAuth>>,
    Extension(ctx): Extension<Arc<BrowserContext>>,
) -> Response {
    let session = match ctx.authenticated() {
        Ok(session) => session,
        Err(error) => return error.into_response(),
    };
    let limits = &auth.config.auth_limits;
    if let Err(response) = auth
        .throttle(
            &ctx,
            "resend",
            &session.identity.id.to_string(),
            limits.recovery_account_ip,
            limits.recovery_ip,
            900,
        )
        .await
    {
        return response.into_response();
    }
    match auth
        .accounts
        .request_verification(session.identity.id)
        .await
    {
        Ok(()) => StatusCode::ACCEPTED.into_response(),
        Err(error) => error.into_response(),
    }
}

async fn forgot_password(
    State(auth): State<Arc<BrowserAuth>>,
    Extension(ctx): Extension<Arc<BrowserContext>>,
    Json(input): Json<Email>,
) -> Response {
    let email = input.email.trim().to_lowercase();
    let limits = &auth.config.auth_limits;
    if let Err(response) = auth
        .throttle(
            &ctx,
            "reset-request",
            &email,
            limits.recovery_account_ip,
            limits.recovery_ip,
            900,
        )
        .await
    {
        return response.into_response();
    }
    if normalize_email(&input.email).is_err() {
        return StatusCode::ACCEPTED.into_response();
    }
    match auth.accounts.request_password_reset(&input.email).await {
        Ok(()) => StatusCode::ACCEPTED.into_response(),
        Err(error) => error.into_response(),
    }
}

async fn reauthenticate(
    State(auth): State<Arc<BrowserAuth>>,
    Extension(ctx): Extension<Arc<BrowserContext>>,
    Json(input): Json<Password>,
) -> Response {
    let session = match ctx.verified() {
        Ok(session) => session,
        Err(error) => return error.into_response(),
    };
    let limits = &auth.config.auth_limits;
    if let Err(response) = auth
        .throttle(
            &ctx,
            "reauthenticate",
            &session.identity.id.to_string(),
            limits.reauthentication_account_ip,
            limits.reauthentication_ip,
            900,
        )
        .await
    {
        return response.into_response();
    }
    match auth.accounts.reauthenticate(session, &input.password).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => error.into_response(),
    }
}

async fn reset_password(
    State(auth): State<Arc<BrowserAuth>>,
    Extension(ctx): Extension<Arc<BrowserContext>>,
    Json(input): Json<ResetPassword>,
) -> Response {
    let limits = &auth.config.auth_limits;
    if let Err(error) = auth
        .throttle(
            &ctx,
            "reset-consume",
            &input.token,
            limits.recovery_account_ip,
            limits.recovery_ip,
            900,
        )
        .await
    {
        return error.into_response();
    }
    match auth
        .accounts
        .reset_password(&input.token, &input.password)
        .await
    {
        Ok(()) => match csrf_response(&auth, None) {
            Ok(mut response) => {
                // Reset is not a login. Clear the requesting browser's cookie,
                // including when it belonged to a different account.
                set_cookie(&mut response, clear_session_cookie(&auth.config));
                response
            }
            Err(error) => error.into_response(),
        },
        Err(error) => error.into_response(),
    }
}

async fn list_keys(
    State(auth): State<Arc<BrowserAuth>>,
    Extension(ctx): Extension<Arc<BrowserContext>>,
) -> Response {
    let session = match ctx.authenticated() {
        Ok(value) => value,
        Err(error) => return error.into_response(),
    };
    match auth.keys.list(session).await {
        Ok(list) => Json(list).into_response(),
        Err(error) => error.into_response(),
    }
}

async fn create_key(
    State(auth): State<Arc<BrowserAuth>>,
    Extension(ctx): Extension<Arc<BrowserContext>>,
    Json(input): Json<crate::unkey::KeyCreate>,
) -> Response {
    let session = match ctx.authenticated() {
        Ok(value) => value,
        Err(error) => return error.into_response(),
    };
    match auth.keys.create(session, input).await {
        Ok(issued) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "key": issued.grant.expose_secret(), "metadata": issued.metadata,
            })),
        )
            .into_response(),
        Err(error) => error.into_response(),
    }
}

async fn revoke_key(
    State(auth): State<Arc<BrowserAuth>>,
    Extension(ctx): Extension<Arc<BrowserContext>>,
    axum::extract::Path(key_id): axum::extract::Path<String>,
) -> Response {
    let session = match ctx.authenticated() {
        Ok(value) => value,
        Err(error) => return error.into_response(),
    };
    match auth.keys.revoke(session, &key_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => error.into_response(),
    }
}
