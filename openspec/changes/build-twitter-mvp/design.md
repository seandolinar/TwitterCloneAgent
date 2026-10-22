# Design

## Context

See `proposal.md` for motivation and the five delta specs for behavior. Inspection found only the OpenSpec setup and README: no source, tests, database, package manifests, or existing capability specs. All application paths below are proposed additions, not descriptions of existing code.

The user confirmed TanStack Start, the elementary social scope, email/password authentication with verification and recovery, Rust/Axum, PostgreSQL, and Docker. Unkey documentation distinguishes API Management from its Compute gateway. Frontline has a documented Docker `local-dev` mode that verifies real keys against hosted Unkey. Its `X-Unkey-Principal` header is unsigned and is trustworthy only behind a protected gateway ingress.

## Goals / Non-Goals

**Goals:**
- Keep business rules and account authorization in one Rust application, reused by browser and integration adapters.
- Make the browser product runnable without cloud credentials while offering a real, opt-in Unkey gateway integration.
- Make ownership, session revocation, key revocation, and proxy trust explicit and testable.

**Non-Goals:**
- Microservices, Redis, a message broker, a separately implemented authentication service, or a local replica of Unkey's control plane.
- An OAuth application ecosystem: users create personal scoped keys, not delegated third-party OAuth grants.
- Automated production deployment or production use of Frontline's local-development mode. No paid account provisioning, image publication, or uploading private code.

## Decisions

### 1. One Rust backend with two authentication adapters

Use Axum/Tokio for HTTP, SQLx for parameterized PostgreSQL queries and migrations, Serde for DTOs, `reqwest` for Unkey HTTPS calls, and maintained RustCrypto/email libraries. Pin compatible versions and commit `Cargo.lock`. Organize `apps/api/` into configuration, HTTP adapters, account/social services, persistence, and external integrations without creating independent services for each.

Run two internal listeners in the same process: browser traffic on port 8080 and gateway traffic on port 8081. Browser routes accept cookie identity only; third-party `/v1` routes accept trusted gateway identity only. Both produce an explicit authenticated actor and call the same service functions. Service functions perform ownership checks rather than relying on hidden UI controls.

**Alternative:** implement authentication and social logic in both TanStack server functions and Rust. Rejected because it creates two authorization authorities. Actix Web is also viable, but Axum's Tower/Tokio middleware ecosystem fits this small async API without a separate execution model.

### 2. TanStack Start is the browser entry point, not a second API authority

Use React/TypeScript, TanStack Start server routes, TanStack Router, and TanStack Query in `apps/web/`, with a committed npm lockfile. Start handles rendering and an allowlisted same-origin proxy:

- Browser `/api/auth/*` maps to the private backend's `/browser/auth/*`.
- Browser `/api/v1/*` maps to `/browser/v1/*` for social operations.
- Only known method/path combinations are forwarded to a fixed backend URL. Normalize and reject encoded traversal; never proxy arbitrary URLs or `/v1` through this entry point.
- Strip inbound `Authorization` and `X-Unkey-*` headers, and replace untrusted forwarding headers. Relay cookies, the actual original `Origin`, the CSRF header, and every `Set-Cookie` response separately. Derive client IP only from the runtime's peer information or explicitly trusted hosting ingress.
- SSR loaders forward the current request's cookies for reads; never store request identity or query caches globally. Disable shared caching for personalized/auth/key-management responses.

Use native `fetch` and typed API DTOs; public OpenAPI types can be generated for TypeScript to reduce drift. Do not use Start's auth features to create an independent session system.

**Alternative:** expose Rust on another public browser origin. Rejected to avoid cross-origin cookie and CORS complexity. A general catch-all reverse proxy is rejected because it could expose the gateway-only listener.

### 3. PostgreSQL owns application and security state

Proposed tables:

| Table | Essential state and constraints |
| --- | --- |
| `users` | UUID, normalized unique email, normalized unique handle, original display handle, password hash, verification time, display name, biography, timestamps |
| `sessions` | Session-token digest, user ID, creation/last-use/absolute expiry, revocation, recent-password-authentication time |
| `account_tokens` | Token digest, user ID, purpose, expiry, consumed time; verification and reset purposes cannot be exchanged |
| `posts` | UUID, author FK, body, creation time; 1–280-code-point check plus non-whitespace validation |
| `follows` | Unique follower/followee pair, FKs, creation time, self-follow prohibition |
| `likes` | Unique user/post pair and FKs; cascade on post deletion |
| `api_keys` | Unkey key ID, immutable owner FK, keyspace, label, masked hint, allowed permissions, expiry, local revoked time, provider cleanup status; no full secret |
| `rate_limit_buckets` | Expiring action/identifier/window buckets, incremented atomically across instances |
| `delivery_jobs` | Account email intents and provider-key-revocation intents, retry time/attempts; no passwords, raw account tokens, or full keys |

Use UTC timestamps and UUIDs. Normalize email by trimming surrounding whitespace and lowercasing without provider-specific dot/plus rewriting; never normalize passwords. Default the display name to the handle. Handles remain fixed for the MVP.

Index author/creation-time/post-ID, following pairs, and reverse follower lookup. Compute counts from indexed relationships initially. Implement a fan-out-on-read home feed using own author ID plus followed IDs, with `(created_at, id)` descending keyset pagination. Encode versioned, strictly validated cursor fields as opaque base64url; a cursor grants no authorization. No global timeline, ranking, precomputed feed, or cache is needed.

**Alternative:** SQLite and in-memory counters would simplify the first demo but weaken concurrent writes and shared security limits. Redis and denormalized counters add operations and consistency states before the product needs them.

### 4. Opaque sessions and purpose-bound one-time account tokens

- Hash passwords with Argon2id, unique salts, and at least OWASP's baseline of 19 MiB memory, two iterations, and parallelism one; benchmark higher parameters within the deployment's resource budget. Run hashing off the async executor with bounded concurrency. Perform comparable dummy-hash work for unknown-email login attempts.
- Use cryptographically random 256-bit session and email tokens and persist only SHA-256 digests. Database-backed session expiry supports immediate logout and recovery revocation without JWT refresh-token machinery.
- Apply the spec's 24-hour idle/seven-day absolute session limits, 24-hour verification-token limit, and 30-minute reset-token limit. Rotate sessions on login. Password login counts as recent reauthentication; additional reauthentication updates that session's timestamp.
- Use production `__Host-session` cookies with HttpOnly, Secure, SameSite=Lax, path `/`, and no Domain. Allow an explicitly development-only non-Secure cookie name on loopback HTTP. Refuse insecure production mode.
- Use a maintained signed double-submit CSRF implementation, bound to the authenticated session or a signed anonymous context before login. A same-origin CSRF bootstrap endpoint supplies the token; unsafe requests require the matching token header and exact configured `Origin`. Refresh the CSRF context after session rotation. Do not synthesize an allowed Origin inside the frontend proxy.
- Submit verification/reset tokens via POST after the link opens a UI page, not a state-changing GET that mail scanners can consume. Use fixed configured application URLs, no arbitrary redirect destinations, no token-bearing analytics, `Referrer-Policy: no-referrer`, and no query/body logging on these routes.
- Consume tokens and update account state transactionally. A successful reset changes the hash, consumes remaining reset tokens, revokes sessions, and marks all owner keys locally revoked while enqueueing provider cleanup in the same transaction.

For email, persist an intent containing only user ID and purpose. A small backend worker generates the raw token in memory, saves its digest, and sends SMTP email; retries generate a replacement token and invalidate the previous token for that purpose. This avoids persisting plaintext token-bearing email bodies in an outbox. Retry jobs with backoff and bounded attempts; the user can request another message. An SMTP failure never changes the current password.

Use the local mail catcher for development and TLS-authenticated SMTP in production. No provider-specific account is chosen.

**Alternative:** JWT browser sessions complicate immediate revocation, while social login or passkeys introduce a different onboarding product than the confirmed email/password MVP. Do not implement password cryptography manually.

### 5. Layered abuse defenses without another datastore

Use atomic PostgreSQL rate-limit buckets shared by all backend instances; hash private email/IP bucket identifiers with a runtime secret. Clean expired buckets and tokens periodically. Treat missing rate-limit storage as a dependency failure, not permission to proceed.

Initial configurable defaults: login and reauthentication five attempts per 15 minutes per email/IP pair plus 50 login attempts per IP; registration five per hour per IP; reset/resend three per 15 minutes per account/IP pair plus 20 per IP; social writes 30 per minute per user across both adapters. Unkey keys have an auto-applied 60-requests-per-minute limit checked once at the gateway. Key-management requests are also throttled. Return 429 and `Retry-After`; do not permanently lock users.

Bound JSON bodies, request duration, database pools, and password-hashing concurrency. Escape user text via normal React rendering, never raw HTML. Apply a production CSP compatible with Start's generated scripts, frame restrictions, content-type protections, and HTTPS/HSTS at the hosting edge. Log request IDs and non-secret operation/error metadata, not headers, credentials, email addresses, or request bodies.

**Alternative:** relying only on process-local middleware or frontend checks fails when there are multiple instances or direct API clients. Unkey's key limits do not replace throttling for password endpoints and browser writes.

### 6. Real Unkey keys and gateway-verified identity

Keep API Management issuance separate from gateway configuration:

1. Configure a development keyspace's `apiId` for key creation and its `keySpaceId` for gateway policy and identity validation; these are different identifiers.
2. Rust calls Unkey's current REST key-management endpoints with a credential scoped only to the application's keyspace and required create/delete operations. Create keys with `externalId` equal to the local user UUID, explicit permissions/expiry, 32 random bytes, an auto-applied request limit, and `recoverable: false`. Do not request encrypted recoverable key storage or broad workspace administration.
3. Return a full key once, only after its local metadata is committed. If the local write fails, do not return the secret, attempt provider cleanup, and never authorize the unregistered key. Do not blindly retry key creation after an ambiguous timeout; report the failure and clean up orphan provider records through a documented reconciliation procedure.
4. Frontline checks Bearer credentials against the configured keyspace. Its successful principal includes identity external ID, key ID, keyspace ID, and granted permissions. Do not call `keys.verifyKey` a second time in Rust: that would add latency and potentially double-count rate limits/credits.
5. Rust validates header version `v1`, type `API_KEY`, expected keyspace, identity external ID, registered key ID/owner, verified account, local expiry and revocation. Enforce the intersection of gateway-granted and locally issued permissions. Missing/unknown/mismatched identity denies access. Never use an arbitrary principal `subject` fallback as a local user ID.
6. Revoke locally first, atomically enqueue provider deletion, and retry cleanup durably. This closes Unkey's documented propagation delay for application access. All protected reads and writes check local key state; no stale application authorization cache.

Use real Frontline in the opt-in `unkey` Compose profile, with a pinned image and TOML configuration matching the documented local-dev contract. It uses a separate verify-only root credential. The default browser stack needs neither root credential and returns a clear integration-unavailable state for key creation.

**Trust boundary:** Frontline strips caller `X-Unkey-*` headers and supplies its own principal. Only Frontline reaches the gateway listener from outside the application's private networks; neither backend listener has a host port. The frontend is a trusted service but must not forward raw caller identities or proxy the gateway listener. Configure distinct browser/gateway/data networks and explicit proxy allowlists, and test header spoofing and path-encoding bypasses. The unsigned principal is not safe on an Internet-accessible origin.

For a future production deployment, the recommended supported boundary is Rust behind Unkey Compute's native gateway, where the origin is reachable only through the gateway. The frontend remains a same-origin browser proxy; use disjoint path policies for cookie routes and `/v1`, with cookie authorization still enforced by Rust. Use private production PostgreSQL and external SMTP. Do not present a `local-dev` Frontline deployment as a production recipe or assume Unkey's managed gateway can front an arbitrary external origin.

**Alternative:** API Management verification inside Axum alone would provide keys, but would not satisfy the requested Unkey gateway. Self-hosting the complete Unkey control plane is disproportionate. Directly trusting principal headers on a public origin is insecure.

### 7. Explicit HTTP surface and permission map

The browser adapter reuses these social shapes under `/api/v1` but authenticates cookies rather than keys. Third-party routes are:

| Third-party route | Permission |
| --- | --- |
| `GET /v1/me`, `GET /v1/profiles/{handle}` | `profiles.read` |
| `PATCH /v1/me` (display name/biography only) | `profiles.write` |
| `GET /v1/profiles/{handle}/posts`, `GET /v1/posts/{id}` | `posts.read` |
| `GET /v1/feed` | `feed.read` |
| `POST /v1/posts`, `DELETE /v1/posts/{id}` | `posts.write`, plus author ownership for deletion |
| `PUT /v1/me/following/{user_id}`, `DELETE /v1/me/following/{user_id}` | `follows.write` |
| `PUT /v1/posts/{id}/like`, `DELETE /v1/posts/{id}/like` | `likes.write` |

Authentication, CSRF bootstrap, reauthentication, and personal key management exist only in the browser adapter. Public browser profile/post reads accept an optional session but never a key. Every `/v1` route requires a key. Serve the public OpenAPI document from `/docs/openapi.json` on the web origin, outside the protected integration namespace.

Use 201 for post/key creation, 204 for successful deletion/idempotent relationship changes, 401 for absent/invalid credentials, 403 for permissions/unverified restrictions, 404 for absent resources, 409 for public handle conflicts, 422 for input validation, 429 for throttling, and 503 for application dependency failures. Return a stable application error envelope with code, safe message, optional field errors, and request ID. Document Unkey gateway errors separately; its edge envelope is not the application's envelope. Frontline outages can produce gateway-specific 5xx rather than the application's exact 503 shape.

Generate OpenAPI from Rust request/response definitions and test permission coverage. Never include emails in public profile DTOs. User text length means Unicode code points, not bytes, UTF-16 units, grapheme clusters, or Twitter's weighted-URL algorithm; use equivalent counting on both sides.

### 8. Container-first execution and evidence

Proposed additions include `apps/api/Dockerfile`, `apps/web/Dockerfile`, `compose.yaml`, `infra/unkey/gateway.toml`, `.env.example`, `.gitignore`, and `tests/`. Use multi-stage non-root application images, a pinned PostgreSQL major, a pinned local mail-catcher image, versioned migrations, and a named database volume. Do not bake environment files or secrets into build contexts.

`docker compose up --build` starts the browser stack, runs migrations through a one-shot initialization command from the backend image, and gates backend readiness on the schema. Add `/healthz` for process liveness and `/readyz` for database/schema readiness on the private browser listener; optional Unkey absence must not make the functioning browser stack unready. Bind web/mail ports to loopback. The `unkey` profile adds a loopback gateway port and fails clearly if its configuration is missing.

Use Rust unit and PostgreSQL integration tests for validation, ownership, concurrent token consumption, idempotency, shared throttling, and recovery revocation. Use frontend component/type checks and Playwright against Compose for real browser-cookie/CSRF, email, social, and cache-isolation workflows. Provider client contract tests can use HTTP fixtures only inside tests, never an insecure application runtime mode. Separately run credentialed smoke tests through actual Frontline with development keys; never claim fixtures prove the live provider works.

**Alternative:** host-only development would be faster to scaffold but would not validate the requested container workflow. Compose is the local delivery target; Kubernetes and automatic cloud deployment are not needed.

## Risks / Trade-offs

- [Unkey credentials and connectivity are required for integration tests] → Keep browser development independent; label live tests separately and report when credentials are absent.
- [Frontline's documented local image is a release candidate and its configuration can change] → Pin a verified image/config pair during implementation and exercise it end to end; do not use `latest` or production claims.
- [Unsigned gateway identity depends on network and proxy correctness] → No published origin port, dedicated ingress, explicit route allowlists, header stripping, and bypass regression tests.
- [PostgreSQL-backed sessions, throttles, and jobs add database load] → Indexed bounded queries, expiring-record cleanup, bounded worker batches, and shared pool limits; optimize only with observed need.
- [No moderation, reporting, account deletion UI, or MFA] → Limit initial access to a controlled MVP; a public launch requires a separate abuse/privacy/security review.
- [Email jobs can deliver an older link before a retry supersedes it] → Make token errors recoverable and resend available; only the newest token for a purpose remains active.
- [Only locally committed revocations are immediately authoritative] → Already-running requests can finish; document this boundary. Test new requests after revocation commits rather than claiming cancellation of in-flight work.

## Migration Plan

There is no existing application data to migrate. During implementation, add an initial versioned schema and validate startup from an empty volume and an already initialized volume. Do not modify the main OpenSpec specs until the change is implemented and explicitly synced/archived.

Deliver local images and operational documentation, not a live deployment. Before any production use, select hosting, configure HTTPS and trusted ingress, inject secrets, supply production SMTP, configure Unkey policies, and test database backup/restore. These are operator prerequisites and require separate authorization for any provisioning or publication.

For rollback, stop the affected release and restore the prior application image only when schema-compatible; otherwise restore a verified database backup. Do not drop tables or delete the named volume as part of ordinary restart or rollback. Document that `docker compose down -v` destroys local data without running it automatically.

## References

Design-time sources; verify pinned versions during implementation:

- TanStack Start server routes: https://tanstack.com/start/latest/docs/framework/react/guide/server-routes
- Unkey API Management key creation: https://www.unkey.com/docs/api-management/keys/creating-keys
- Unkey verification and propagation delay: https://www.unkey.com/docs/api-management/keys/verifying-keys
- Unkey gateway local development: https://www.unkey.com/docs/compute/gateway/local-development
- Unkey gateway API-key policy: https://www.unkey.com/docs/compute/gateway/api-key-auth
- Unkey principal and its trust boundary: https://www.unkey.com/docs/compute/gateway/principal
