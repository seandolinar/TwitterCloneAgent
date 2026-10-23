# Rust API

## Optional email

Set `EMAIL_ENABLED=false` for the email-free MVP, including production. Register
and log in with an email/password immediately; email is a login identifier, not
verified ownership. Social actions, feeds, reauthentication and key management
permit these accounts while retaining session, CSRF, password, owner and key-scope
checks. No verification mail is queued and the SMTP worker is not started.
Existing mail intents remain paused. SMTP settings are ignored in this mode.

`GET /browser/auth/settings` returns `{ "emailEnabled": false }`; session DTOs
add `emailEnabled` and retain the true stored `verified` status. Verification,
resend, reset requests and reset consumption return 503 / `email_disabled`.
No email password recovery is available. Existing mailbox verification timestamps
are never fabricated or overwritten. Setting `EMAIL_ENABLED=true` (also the
default when omitted) restores verification gates and email/recovery behavior;
configure SMTP and recreate containers before enabling it in production.

The backend has separate browser and gateway listeners. Neither is intended to
be publicly reachable directly. Browser account endpoints, `/healthz` and
`/readyz`, schema migrations, and configuration checks are implemented.
The browser social/key APIs and private gateway social adapter are implemented.
Password reset's immediate key-denial and cross-adapter write limits are tested
through application requests; live provider acceptance remains separate.

## Local toolchain

The repository pins Rust in `rust-toolchain.toml`. With rustup installed:

```sh
cargo check --manifest-path apps/api/Cargo.toml
cargo test --manifest-path apps/api/Cargo.toml
cargo clippy --manifest-path apps/api/Cargo.toml --all-targets -- -D warnings
```

If you already have an independent development PostgreSQL instance, migrate
before running the current scaffold without containers. First export the same
private `CSRF_SECRET` and `RATE_LIMIT_SECRET` values described in the root setup
instructions into your shell; they are required for both commands below:

```sh
DATABASE_URL=postgres://localhost/twitterclone \
  cargo run --manifest-path apps/api/Cargo.toml -- migrate
DATABASE_URL=postgres://localhost/twitterclone \
  cargo run --manifest-path apps/api/Cargo.toml
```

Alternatively use the root Compose setup, which supplies PostgreSQL and runs its
one-shot migration service automatically. The standard Compose database is not
published to your host. Default standalone listeners are loopback-only:
browser `127.0.0.1:8080`, gateway
`127.0.0.1:8081`. The gateway listener exposes no protected routes until its
authorization adapter is implemented; it does not accept forged identities.

`/healthz` never queries the database. `/readyz` is bounded to one second and
returns 503 if PostgreSQL is unreachable, migration checksums/versions do not
match the binary, a migration failed, or required tables are missing. The
`healthcheck` command queries `/readyz` and is used by Compose. The API refuses
initial startup if its schema is not ready; it never silently runs migrations
inside an HTTP request. Migration and connection failures use safe messages,
not connection strings.

## Runtime configuration

The process reads its environment; it does not load `.env` itself. Compose reads
the root `.env` and supplies these settings to the API and migration service.

| Setting | Behavior |
| --- | --- |
| `APP_ENV` | `development` (default) or `production`; other values are rejected |
| `APP_ORIGIN` | Exact external browser origin; default `http://localhost:3000`, no path/query/credentials |
| `BROWSER_BIND`, `GATEWAY_BIND` | Distinct IP/port pairs; defaults above |
| `DATABASE_URL` | Required PostgreSQL connection URL; never printed |
| `COOKIE_SECURE` | Defaults true in production, false in development; false requires a loopback development origin |
| `CSRF_SECRET`, `RATE_LIMIT_SECRET` | Required in every environment; distinct, nonblank private values of at least 32 bytes; never printed |
| `EMAIL_FROM` | Single sender email address; defaults to `noreply@example.test` only in development; production requires a non-development sender |
| `SMTP_HOST`, `SMTP_PORT` | Default `mail:1025` for local email capture |
| `SMTP_TLS` | Exact `true`/`false`; true requires STARTTLS (normally port 587); false is development-only |
| `SMTP_USERNAME`, `SMTP_PASSWORD` | Supplied together when set; required in production; never printed |
| `UNKEY_MANAGEMENT_ROOT_KEY`, `UNKEY_API_ID`, `UNKEY_KEYSPACE_ID` | Optional as a group; API IDs start `api_`, keyspace IDs start `ks_`; partial configuration fails |

Production additionally requires HTTPS, nonzero loopback Rust bindings, external
authenticated SMTP, and an external authenticated PostgreSQL URL with
`sslmode=verify-full`. Local SMTP, unverified database TLS, duplicate SSL modes,
and security-relevant URL aliases are rejected. Compute mode additionally
validates platform/internal ports, exact loopback upstream URLs, HOST, and
required environment-specific Unkey configuration. See
`docs/deployment/unkey-compute.md` for the combined image and configuration.
Successful validation is not evidence of a live production deployment.

Generate each security secret independently with `openssl rand -hex 32` in your
own terminal. Keep the configured values stable across restarts and shared by
every backend instance. The backend preserves them exactly, with no generated,
hardcoded, or development-only fallback; invalid configuration fails before
connecting to PostgreSQL or serving requests. Do not log, commit, or share the
values in chat. Test harnesses generate disposable runtime-only values instead.

Requests have a server-generated UUID in `X-Request-Id`; client IDs are not
trusted. Error responses contain `error.code`, a safe `error.message`, and the
matching `requestId`. Browser ingress strips `X-Unkey-*` headers. Responses
disable caching and MIME sniffing. The backend does not log URLs, query strings,
request bodies, credential headers, or configuration values.

Each listener limits bodies to 16 KiB, active requests to 64, and handler duration
to ten seconds. Oversized bodies return 413, slow handlers 408, and overload 503.
Both listeners handle SIGINT/SIGTERM for graceful shutdown. These HTTP bounds
are separate from the shared account/user rate limits.

## Shared throttling infrastructure

The shared limiter uses an atomic PostgreSQL upsert per attempt. Windows are
aligned to PostgreSQL's clock, not an individual instance's clock. Independent
backend instances with the same configuration share one counter; there is no
process-local counter or storage-outage fallback. A fixed window can allow
requests on both sides of its boundary; it is not a rolling-window algorithm.

Identifiers are normalized by the calling service, then stored only as
domain-separated HMAC-SHA-256 digests using `RATE_LIMIT_SECRET`. Actions and
positive limit/window settings are server-owned. Limits cannot be disabled by
passing zero values. Rejected requests return 429 with a positive `Retry-After`;
storage errors/timeouts fail closed with 503. Queries are bounded to two seconds.

Each backend sweeps at most 1,000 expired buckets every minute, using
`SKIP LOCKED` so replicas can clean up safely without communicating. Active
buckets are retained, and shutdown stops the maintenance task.

Authentication endpoints now use the shared limiter before credential/hash work.
Defaults are five email/IP login attempts per 15 minutes plus 50 per IP;
registration five per hour per IP (also checked per email/IP); reset requests
and verification resends three per account/IP per 15 minutes plus 20 per IP;
reauthentication five per user/IP per 15 minutes plus 50 per IP. Verification
consumption uses the recovery limits with a private token/IP identifier.
Each operation has a distinct budget. Failures consume attempts; CSRF/origin
rejections do not. Public forwarding headers never select the limit bucket.

Override positive attempt counts with `AUTH_LOGIN_ACCOUNT_IP_LIMIT`,
`AUTH_LOGIN_IP_LIMIT`, `AUTH_REGISTRATION_IP_LIMIT`,
`AUTH_RECOVERY_ACCOUNT_IP_LIMIT`, `AUTH_RECOVERY_IP_LIMIT`,
`AUTH_REAUTH_ACCOUNT_IP_LIMIT`, and `AUTH_REAUTH_IP_LIMIT`. Empty/unset uses
the defaults; zero/negative/invalid values fail startup. Windows remain fixed.
Use identical settings across replicas. Social writes use the separate shared
user budget documented below; issued keys carry auto-applied Unkey gateway
limits whose live enforcement remains unverified under task 9.3.

Run shared-limit integration tests with a disposable database:

```sh
python3 tests/with_postgres.py cargo test \
  --manifest-path apps/api/Cargo.toml --test rate_limits -- --ignored
```

The tests send concurrent HTTP requests to two application instances with
independent connection pools, verify a single shared budget and 429 headers,
exercise expiry and bounded cleanup, and prove unavailable storage denies access.

## Durable delivery intents

The queue stores only typed verification-email, password-reset-email, and
provider-key-revocation intents: user UUIDs or Unkey key IDs, not raw tokens,
passwords, API keys, email addresses, or token-bearing message bodies. Enqueue
inside the same transaction as the corresponding account/key change so failures
cannot leave a partial business operation.

Claims use PostgreSQL `SKIP LOCKED`, a 30-second lease, and a fresh claim ID.
Multiple workers cannot simultaneously own a live claim. A crashed worker's
expired claim can be reclaimed; an old worker cannot acknowledge or reschedule
it afterward. Token/key metadata work can lock and validate its claim in the
same transaction before making changes.

There are at most eight claims per intent. Retry delays start at five seconds
and double, up to 640 seconds. Exhausted intents remain for inspection and do
not retry forever; recovery must be deliberate after the dependency problem is
fixed. Successful acknowledgement removes the intent. Delivery is **at least
once**, so drivers must tolerate replay; this is not a claim of exactly-once SMTP.

A delivery batch runs at most four callbacks concurrently with a ten-second
callback timeout, below the lease duration. Queries and connection acquisition
are bounded to two seconds. The generic queue delegates to real drivers, never
a runtime mock or success-without-delivery fallback. The email worker now drains
only email intents; unsupported provider-cleanup intents remain untouched until
the Unkey driver is implemented.

Run queue recovery/concurrency tests in disposable per-test databases:

```sh
python3 tests/with_postgres.py cargo test \
  --manifest-path apps/api/Cargo.toml --test jobs -- --ignored
```

Every integration test creates a fresh database only after verifying the
wrapper-owned parent database name. Tests cannot consume each other's queue
items, and all databases disappear when the wrapper removes its disposable
PostgreSQL container/anonymous volume.

## Registration and credential services

Registration trims/lowercases email addresses without rewriting provider aliases,
rejects display-name/multiple-address input, and preserves the requested handle's
case. Handles must be 3–30 ASCII letters, digits, or underscores; the database
enforces case-insensitive uniqueness. New accounts are unverified and their
verification intent is committed atomically with the account.

An existing email receives the same service acknowledgement as a new account,
without replacing its password/handle or adding another registration intent.
Public handle conflicts are reported as conflicts. Credential checks return the
same safe failure for an unknown email and a wrong password; unknown emails still
perform a real verification against a startup-generated dummy hash. The dummy
password can never authenticate a nonexistent account.

Passwords are 15–128 Unicode code points, never trimmed/truncated. RustCrypto
Argon2id v19 uses 19,456 KiB, two passes, one lane, and a fresh OS-generated salt.
At most two blocking hash/verification jobs run per shared service instance;
clones share that budget, and a cancelled async request does not release a
still-running blocking job's permit. Overload fails safely rather than spawning
unbounded CPU work. Stored hash parameters are checked before verification to
avoid unbounded resource allocation; future parameter changes need a deliberate
compatibility/rehash plan.

Plaintext inputs are not stored or logged. Owned blocking-task password buffers
are zeroized; hash-bearing internal rows/services do not implement diagnostic
formatting. Public identities contain only user ID, handle, and verification
status, not the email or hash.

These services are exposed only through the private browser adapter with
CSRF/origin checks and shared endpoint throttles. The allowlisted browser proxy
and onboarding/recovery screens are implemented; see `../web/ACCOUNT_GUIDE.md`.

```sh
python3 tests/with_postgres.py cargo test \
  --manifest-path apps/api/Cargo.toml --test accounts -- --ignored
```

## Verification and recovery email

The backend polls durable email intents every two seconds. It locks the account,
validates its job lease, generates a 256-bit OS-random token in memory, and stores
only its SHA-256 digest. Replacement/retry invalidates the previous token for that
purpose. Verification links expire within 24 hours; reset links within 30 minutes.
SMTP delivery failure does not change passwords, verification state, or sessions.

Mail uses the configured origin with fixed `/verify-email` and `/reset-password`
paths and plain-text bodies. Opening a link is not a state-changing GET; the UI
must submit it via a protected POST. The verification service consumes a
purpose-bound, unexpired token transactionally, with account-first lock ordering
and a conditional consume so concurrent requests cannot use it twice.

Only local development permits plaintext SMTP to the mail catcher. Production
requires an operator-provided sender, external authenticated SMTP, and mandatory
STARTTLS with certificate verification. Implicit SMTPS on port 465 is not the
configured transport mode; use a provider's STARTTLS endpoint. The app does not
log token-bearing URLs, message bodies, or underlying transport credentials.

The default live foundation check now delivers a real verification message to
Compose Mailpit and verifies its digest-backed token without printing it. Domain
tests cover expiry, replay, wrong purpose, concurrent consumption, resend,
generic recovery requests, and SMTP failure. Password replacement and local
credential revocation are covered by the recovery/gateway tests below. Cleanup
and local gateway admission use fixture-tested real adapters; live provider
acceptance remains pending.

```sh
python3 tests/with_postgres.py cargo test \
  --manifest-path apps/api/Cargo.toml --test email -- --ignored
```

## Opaque browser sessions

Successful login creates a fresh 256-bit token and persists only its digest.
If a previous browser token is supplied, successful login revokes it in the same
transaction; failed credentials leave it unchanged. Credential checking and
issuance lock the account so password recovery cannot race an old-password login
into creating a session after recovery revokes sessions.

Each authenticated lookup checks revocation, seven-day absolute expiry, and
24-hour inactivity against PostgreSQL's clock. Activity renews only the idle
timestamp, never absolute expiry. Missing/invalid/expired tokens do not establish
identity; storage errors fail closed rather than becoming anonymous access.
Logout is idempotent and server-revokes the token; the HTTP adapter also clears
its cookie using the same name/path attributes.

Production uses `__Host-session` with HttpOnly, Secure, SameSite=Lax, path `/`,
and no Domain. Explicit loopback HTTP development uses
`twitterclone_dev_session` without Secure. Raw session grants have no diagnostic
or serialization implementation; public session identity contains no raw token
or digest. CSRF binding can use the private session digest without exposing it.

Login counts as recent password authentication. Reauthentication checks the
current password and live session under the account lock, then refreshes the
five-minute window; a stale/revoked session cannot be revived by cached context.
The shared maintenance loop prunes bounded batches of expired/revoked/idle
sessions and consumed/expired account tokens, preserving active records.

```sh
python3 tests/with_postgres.py cargo test \
  --manifest-path apps/api/Cargo.toml --test sessions -- --ignored
```

The tests cover rotation, failed-login preservation, logout/revocation,
idle/absolute expiry, idle renewal without absolute extension, reauthentication,
cookie attributes/removal, maintenance, and unavailable storage.

## Browser security adapter

`GET /browser/auth/csrf` returns `{ "csrfToken": "..." }` and an HttpOnly
CSRF-context cookie. Unsafe browser methods must send `X-CSRF-Token`, that
cookie, and exactly the configured external `Origin` (no trailing slash).
Missing, duplicate, `null`, or hostile origins/tokens are rejected with 403.
There is no permissive CORS or Referer fallback.

Signed double-submit pairs use the pinned `csrf` library, expire after one hour,
and are cryptographically bound to the live session digest, or to a signed random
anonymous context before login. Login rotates both the session and CSRF pair;
logout clears the session and returns a new anonymous pair. Clients must replace
their in-memory token with the returned `csrfToken`, or bootstrap again after
session changes. Production CSRF cookies use `__Host-csrf`, Secure, HttpOnly,
SameSite=Lax, path `/`, no Domain; loopback development uses
`twitterclone_dev_csrf` without Secure. Bootstrap responses are `no-store`.

Browser identity comes only from the live session cookie; Bearer and
`X-Unkey-*` headers cannot establish it. The shared request context exposes
authenticated/verified guards; unverified users can inspect their session,
resend verification, and log out, but cannot reauthenticate for protected actions.
Social routes use that same layer; their services enforce verification and
ownership. Future key routes must use it too.

Client IP is a private ingress contract: `X-Twitterclone-Client-IP` may be
supplied only by the trusted frontend, which must overwrite caller values from
trusted peer/hosting context. Without it, Rust uses its socket peer. Missing
peer context fails closed. `Forwarded`/`X-Forwarded-For` never establish identity
here. Neither Rust listener may be published. Frontend boundary tests separately
exercise raw path rejection and trusted socket-derived IP forwarding; see
`../web/README.md`.

```sh
python3 tests/with_postgres.py cargo test \
  --manifest-path apps/api/Cargo.toml --test browser_auth -- --ignored
```

These HTTP adapter tests cover login CSRF, missing/hostile/duplicate origins
and tokens, session-bound pair transplantation, forged identities, unverified
reauthentication, safe verification POSTs, rotation/logout, shared endpoint
limits, spoofed public forwarding headers, and storage-outage denial.

## Browser social HTTP contract

The private cookie listener exposes these routes under `/browser/v1`.
The browser proxy maps public `/api/v1` to this namespace; `/v1` is
reserved for the separate key-authenticated integration listener. Sending a
Bearer key or `X-Unkey-*` header here never establishes a user.

| Method and private route | Request | Success |
| --- | --- | --- |
| `GET /browser/v1/me` | Session cookie | 200 Profile |
| `PATCH /browser/v1/me` | `{ "displayName": "Alice", "biography": "Hello" }` | 200 updated Profile |
| `GET /browser/v1/profiles/{handle}` | Case-insensitive handle | 200 Profile |
| `GET /browser/v1/profiles/{handle}/posts` | Optional `limit`, `cursor` | 200 Page |
| `GET /browser/v1/feed` | Verified session; optional `limit`, `cursor` | 200 Page |
| `POST /browser/v1/posts` | `{ "body": "Hello" }` | 201 Post |
| `GET /browser/v1/posts/{id}` | Post UUID | 200 Post |
| `DELETE /browser/v1/posts/{id}` | Author session | Empty 204 |
| `PUT /browser/v1/me/following/{id}` | Target user UUID; no body | Empty 204 |
| `DELETE /browser/v1/me/following/{id}` | Target user UUID; no body | Empty 204 |
| `PUT /browser/v1/posts/{id}/like` | Post UUID; no body | Empty 204 |
| `DELETE /browser/v1/posts/{id}/like` | Post UUID; no body | Empty 204 |

Every unsafe method requires the exact Origin and CSRF context described above.
Only authenticated verified users can mutate or read a personalized feed.
Anonymous and unverified visitors may read public profiles/posts/profile lists;
`me` returns the same public profile shape for a valid session, not private
account fields. No caller-selected author/owner/handle is accepted in write DTOs;
unknown fields are rejected. There is no route for editing someone else's profile.
Deleting a missing/already deleted post returns 404; deleting another author's
post returns 403. Post deletion cascades likes. Follow/like changes are
idempotent, but still consume write attempts. Self-follow is 422; absent targets
and likes/unlikes of absent or deleted posts are 404.

Profile JSON:

```json
{
  "id": "<user-uuid>",
  "handle": "Alice",
  "displayName": "Alice",
  "biography": "Hello",
  "joinedAt": "2026-01-01T00:00:00Z",
  "followerCount": 0,
  "followingCount": 0,
  "postCount": 1,
  "viewerFollows": false
}
```

Post JSON:

```json
{
  "id": "<post-uuid>",
  "authorId": "<user-uuid>",
  "authorHandle": "Alice",
  "authorDisplayName": "Alice",
  "body": "Hello",
  "createdAt": "2026-01-01T00:00:00Z",
  "likeCount": 0,
  "viewerLikes": false
}
```

Page JSON is `{ "items": [<Post>, ...], "nextCursor": "<opaque-cursor-or-null>" }`.
`nextCursor` is JSON `null` when no further page exists, otherwise a string.
Pass it unchanged in the next request's URL-encoded `cursor` parameter. Pages
default to 20, accept 1–100, and sort `(createdAt, id)` descending. The feed
contains only own/currently-followed authors; unfollowing changes future pages.
Cursors encode strictly validated versioned ordering fields, confer no identity
or authorization, and remain usable if their anchor post is deleted. Inserts
ahead of a cursor do not repeat earlier items. Invalid cursor/limit values
return 422 `invalid_pagination`; malformed/unknown query fields or UUID path
syntax return safe 400 errors.

Display names contain 1–50 Unicode code points; biographies at most 160.
Posts contain 1–280 code points and at least one non-whitespace character.
Text is preserved, not trimmed, interpreted as HTML, or counted as UTF-16 units.
Clients must render it as text. Counts are computed from current indexed
relationships, and authenticated responses include viewer relationship flags.

All social writes share **one** PostgreSQL budget per user across operation
types and service instances: 30 attempts per minute by default, configurable via
positive `SOCIAL_WRITES_PER_MINUTE`. Rejections use the same safe application
envelope and no-store/referrer protections as account endpoints; 429 includes
`Retry-After`, and dependency failure never grants access. HTTP tests cover
anonymous/unverified/owner/other-user authorization, forged principal rejection,
spoofed ownership fields, CSRF, idempotent operations, and shared browser budgets.
Gateway tests additionally exercise permission intersections, resource ownership,
and the same per-user budget across both actual application adapters.

```sh
python3 tests/with_postgres.py cargo test \
  --manifest-path apps/api/Cargo.toml --test social --test browser_social -- --ignored
```

## Account HTTP contract

All paths below belong to the **private cookie listener**, not `/v1`. The
public browser proxy maps `/api/auth/*` to `/browser/auth/*`. There is no API-key
authentication on account routes.
JSON request fields are exact, case-sensitive, and reject unknown fields.
Passwords are never trimmed. Send `Content-Type: application/json` for JSON bodies.
Every POST requires the original exact `Origin`, CSRF header and CSRF cookie;
authenticated actions additionally require the session cookie.

| Method and private path | Request | Success |
| --- | --- | --- |
| `GET /browser/auth/csrf` | No body; optional session cookie | 200 `{ "csrfToken": "..." }` and CSRF cookie |
| `GET /browser/auth/session` | Session cookie | 200 `{ "id": "<uuid>", "handle": "Alice", "verified": false, "recentPasswordAuth": true, "emailEnabled": true }` |
| `POST /browser/auth/register` | `{ "email": "alice@example.test", "handle": "Alice", "password": "a long example password" }` | Empty 202, including duplicate email; no automatic login |
| `POST /browser/auth/login` | `{ "email": "alice@example.test", "password": "a long example password" }` | 200 `{ "csrfToken": "..." }` plus **two separate** Set-Cookie headers for rotated session and CSRF context |
| `POST /browser/auth/logout` | No body | 200 `{ "csrfToken": "..." }`, removed session cookie and new anonymous CSRF context; idempotent with refreshed context |
| `POST /browser/auth/verify-email` | `{ "token": "<email-link-token>" }` | Empty 204; does not create a session |
| `POST /browser/auth/resend-verification` | No body; session required | Empty 202, including already verified accounts |
| `POST /browser/auth/forgot-password` | `{ "email": "alice@example.test" }` | Empty 202 whether or not the address exists |
| `POST /browser/auth/reset-password` | `{ "token": "<email-link-token>", "password": "a new long example password" }` | 200 `{ "csrfToken": "..." }` and removed session cookie; explicit fresh login required |
| `POST /browser/auth/reauthenticate` | `{ "password": "a long example password" }`; verified session required | Empty 204; extends recent-password window to five minutes |

`GET /browser/auth/session` returns 401 for absent/revoked/expired cookies. Its
DTO contains no email, hashes, bearer values, or token digests. Login failures
share 401 `invalid_credentials`. Validation errors use 422, public handle
conflicts 409, CSRF or unverified restrictions 403, limits 429 with a positive
`Retry-After`, and dependency failures 503. Malformed JSON/unsupported content
types use 400/415; unknown fields or wrong JSON shapes use 422.

Application failures always use the safe envelope below. The request ID is
server-generated and matches `X-Request-Id`; raw parser/database/provider
diagnostics and submitted input are not echoed:

```json
{
  "error": { "code": "invalid_token", "message": "This link is invalid or has expired." },
  "requestId": "<server-generated-uuid>"
}
```

All responses use `Cache-Control: no-store`, `X-Content-Type-Options: nosniff`,
and `Referrer-Policy: no-referrer`. Never put passwords or link tokens in API
query parameters; submit link tokens in POST bodies after opening the UI page.
Do not log request headers/bodies/query strings, run token-bearing analytics,
or persist session/password/CSRF/API-key secrets in browser storage.

Example client sequence (values are placeholders, not working credentials):

```text
GET /browser/auth/csrf
  -> save the CSRF cookie; keep response.csrfToken in memory
POST /browser/auth/login
  Origin: http://localhost:3000
  Content-Type: application/json
  Cookie: twitterclone_dev_csrf=<cookie-from-bootstrap>
  X-CSRF-Token: <token-from-bootstrap>
  {"email":"alice@example.test","password":"a long example password"}
  -> save each Set-Cookie independently; replace the in-memory csrfToken
GET /browser/auth/session
  Cookie: <current cookies>
POST /browser/auth/logout
  Origin: http://localhost:3000
  Cookie: <current cookies>
  X-CSRF-Token: <current in-memory token>
  -> save cookie removal/new CSRF cookie; replace the in-memory csrfToken
```

The browser HTTP tests exercise this sequence and the documented JSON/status
shapes, malformed input, duplicate-email acknowledgements, unknown-email versus
wrong-password failure, and absence of submitted secrets from error/session
payloads. A real-process test checks that token-bearing failed requests do not
leak into captured application stdout/stderr.

### Email delivery and recovery limitations

The default Compose SMTP service is `mail:1025`, with mail inspection at the
configured loopback mail UI (normally http://localhost:8025). Production needs
an external STARTTLS SMTP endpoint, operator-supplied sender, and both SMTP
credentials. See runtime configuration and email-worker sections above.
Opening email links does not consume them; a protected POST does. Only the
latest delivered/retried token for each purpose stays active: verification
tokens expire within 24 hours, password-reset tokens within 30 minutes.
Session inactivity expires after 24 hours and absolute lifetime after seven
days. CSRF contexts expire after one hour. Password reauthentication lasts
five minutes.

Email delivery is at least once with bounded durable retry. Delayed/retried
mail can supersede an earlier link; request a fresh email if a link expired or
was used. Lost access to the email inbox has no administrator/MFA recovery
fallback in this MVP. Password reset does not implicitly verify an account,
does not log it in, and does not rotate the independently managed application
security secrets. Provider cleanup may lag local key revocation; running
requests are not cancelled. Immediate application key denial is covered by
gateway tests; hosted-provider acceptance remains separately tracked in 9.3.

For production, require HTTPS, Secure host-only cookies, private listeners,
exact configured origin, independently generated stable security secrets shared
across replicas, authenticated SMTP, and certificate-verified database TLS.
Local combined-image/TLS/failure tests are implemented; real external SMTP,
hosted-provider acceptance, and any actual deployment remain unverified.

## Atomic password recovery

`POST /browser/auth/forgot-password` with `{ "email": "..." }` returns the same
empty 202 acknowledgement for registered, unknown, and malformed addresses
(subject to the same endpoint throttles). Only registered addresses queue an
intent; a failed delivery never replaces the password or revokes sessions.

`POST /browser/auth/reset-password` accepts `{ "token": "...", "password": "..." }`
under the same Origin/CSRF rules. It enforces the registration password policy
without trimming. A valid reset transaction consumes the purpose-bound token,
replaces the password hash, consumes remaining reset tokens, revokes every owner
session, locally revokes every owner key, and inserts durable provider-deletion
intents for newly revoked keys. Account-first locking serializes resets with
login/reauthentication and token delivery; racing consumption has one winner.
Any failure, including cleanup enqueue failure, rolls back every change.

Success returns 200 with a new anonymous `csrfToken` and clears the requesting
browser's session cookie. It does not log the user in. Expired, consumed, and
wrong-purpose tokens return the same safe 422 error; an invalid replacement
password does not consume a valid token. Other accounts' sessions/keys are
untouched. Already-running requests may finish; revocation governs new requests
after commit.

```sh
python3 tests/with_postgres.py cargo test \
  --manifest-path apps/api/Cargo.toml --test recovery -- --ignored
```

The tests prove old-password/session rejection, exact replacement-password
whitespace, local key-state denial, queued cleanup without provider access,
wrong-purpose/expired/replayed token rejection, reset/login races, and full
rollback on a late enqueue error. The HTTP tests additionally cover generic
acknowledgements, reset CSRF, cookie clearing, and forced fresh login.
Gateway request tests now replay an unchanged, previously valid principal after
reset commits and verify that new protected reads and writes fail immediately.
They prove local application denial despite cached provider identity, not live
Unkey authentication or provider propagation timing.

## Unkey management foundation

The fixed-origin, bounded Unkey API Management client, browser key endpoints,
and durable cleanup driver are implemented with local HTTP/PostgreSQL contract
tests. See [the management
contract](UNKEY.md) for the separate `api_`/`ks_` IDs, scoped credentials,
non-recoverable creation, configured auto-applied limits, uncertain creation
handling, soft-deletion semantics, and the private gateway permission boundary.
The browser key settings screens are implemented; successful one-time key UI
flows use explicit transport fixtures. No live provider key was created or
verified as part of these checks. See the [final local validation
evidence](../../docs/validation/twitter-mvp.md).
