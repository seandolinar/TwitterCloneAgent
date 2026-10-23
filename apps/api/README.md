# Rust API

The backend has separate browser and gateway listeners. Neither is intended to
be publicly reachable directly. Browser `/healthz` and `/readyz`, schema
migrations, and configuration checks are implemented. Account, social, and key
routes are still pending.

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

Production additionally requires HTTPS and external authenticated SMTP, and
rejects the local mail catcher configuration. Successful configuration validation
alone is not evidence that the unfinished scaffold is production-ready.

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

This implements the shared counter/error/cleanup infrastructure. Authentication
and social endpoints are still pending; their policy wiring belongs to the
corresponding tasks. Planned defaults remain five account/IP login attempts per
15 minutes plus 50 per IP, and 30 social writes per minute per user. Per-key
Unkey gateway limits remain a separate integration.

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

These are backend primitives, **not exposed account HTTP endpoints yet**.
Sessions, CSRF, and endpoint throttles are wired by subsequent tasks before
browser onboarding is available.

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
generic recovery requests, and SMTP failure. Password replacement/session/key
revocation remains a separate pending recovery task.

```sh
python3 tests/with_postgres.py cargo test \
  --manifest-path apps/api/Cargo.toml --test email -- --ignored
```
