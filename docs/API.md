# Social API contract and executable examples

The generated OpenAPI 3.1 document is served publicly on the **browser origin**
at `/docs/openapi.json` (locally `http://localhost:3000/docs/openapi.json`).
It contains all twelve third-party operations, not browser account endpoints.
Its example server is the separate optional Frontline origin,
`http://localhost:8088`; override it for your environment. The ordinary browser
port deliberately rejects `/v1`. No key is required to download the document.

## Generate and check

Schemas come from the actual Rust request/response DTOs with `utoipa::ToSchema`;
operations are annotated on the shared HTTP handlers. Required permissions are
injected from the same catalogue used by gateway authorization, not a second
permission matrix. A test compares the committed document with generation and
checks every catalogued operation reaches an authenticated runtime route.

From the repository root:

```sh
cargo run --quiet --manifest-path apps/api/Cargo.toml --bin openapi \
  > apps/web/public/docs/openapi.json
cargo test --manifest-path apps/api/Cargo.toml --lib openapi::tests
npm --prefix apps/web run test:proxy
```

Generation needs no database, environment secrets, or Unkey connection. The
document is committed and included in the web image; regenerate after changing
DTOs, handlers, or permissions. Build/typecheck alone does not check drift.

## Authentication, validation, and pagination

Send `Authorization: Bearer <personal-key>` to the configured gateway. Never use
a management/verification root credential as a personal key. Each operation's
`x-required-permission` gives its required scope. All `/v1` reads require a key
even if the equivalent browser resource is public. Effective grants are the
intersection of provider and local permissions; ownership is checked separately.
Default keys have `profiles.read`, `posts.read`, and `feed.read` only.

POST `/v1/posts` accepts `{"body":"Hello from the API"}` and returns HTTP 201
with the `Post` DTO, including its UUID, authenticated author, creation time,
and like state. Body length is 1–280 Unicode code points with at least one
non-whitespace character. JSON unknown fields (including spoofed author IDs)
are rejected. Profile updates require both `displayName` (1–50 code points)
and `biography` (0–160); handles cannot be changed. All content is plain text.

Feed and profile-post pages return `{"items":[...],"nextCursor":null}` or a
non-null opaque `nextCursor`. Pass that exact string as the next `cursor` query
parameter, using normal query encoding. `limit` defaults to 20 and is 1–100.
Ordering is descending creation time then ID. Inserts between pages do not
repeat earlier results; this is not a frozen snapshot. An empty page or null
cursor is the end. A feed contains only your own and currently followed authors.

## Application versus gateway errors

Rust application errors have this shape (illustrative, non-secret values):

```json
{
  "error": {
    "code": "permission_denied",
    "message": "This key does not have the required permission."
  },
  "requestId": "00000000-0000-4000-8000-000000000001"
}
```

`X-Request-ID` matches `requestId`. The generated responses describe application
codes/statuses: invalid key 401; missing permission/ownership 403; absent resource
404; invalid input 422; throttling 429 with `Retry-After` seconds; unavailable
authorization storage 503. Parser failures use a safe `request_failed`, not
framework internals. Private responses are `no-store`. No email, credential,
request body, or raw provider error is reflected in these errors.

**Frontline can reject before Rust runs.** Its provider-owned error body is not
the application envelope. For example, the pinned real image's missing-key
response is HTTP 401 with a `missing_credentials` marker, not an application
`invalid_key`/`requestId`. Invalid keys, key limits, and upstream verification
outages may produce gateway-specific 401/403/429/5xx. Do not parse every failure
as `ErrorEnvelope`, or assume a gateway failure has an application request ID.
The OpenAPI `x-gateway-errors` extension describes this distinction without
inventing a stable provider schema. Consult the pinned
[gateway contract](https://www.unkey.com/docs/compute/gateway/api-key-auth).

Honor `Retry-After` where supplied. Rust does **not** verify a key again with
Unkey or consume a second per-key limit. Shared per-user writes are a separate
application budget across browser and API. Never blindly retry POST after
timeouts/5xx: a lost response does not prove the post was not committed. Refetch
the feed to reconcile. Known-target PUT/DELETE relationship operations are
idempotent, but still consume write attempts and may fail authorization.

## Run the executable requests

`apps/api/examples/social_api.rs` is the complete executable example. It sends
real HTTP requests and checks:

1. Two `POST /v1/posts` requests with a write key return 201.
2. `GET /v1/feed?limit=1`, then the returned cursor, produce distinct posts.
3. The same post creation with a read-only key returns 403 `permission_denied`.
4. `/v1/auth/keys/{id}` does not exist, even with a write key.
5. A **verified browser session** bootstraps a CSRF pair from `/api/auth/csrf`
   and sends `DELETE /api/auth/keys/{id}` with the actual browser Origin.
6. A new request using that revoked read key fails, then the two example posts
   are deleted using their known IDs and the separate write key.

Use a disposable development account, two separate personal keys belonging to
that account, and its logged-in browser session. Give the write key
`posts.write` (and any desired read permissions); give the read key the default
read-only scopes. The example **revokes the read key** and writes/deletes two
posts. It does not create keys, alter passwords, or automatically retry.

Supply `WRITE_KEY`, `READ_KEY`, `READ_KEY_ID`, and `SESSION_COOKIE` privately
through your local environment, not in tracked files, command-line arguments,
chat, or shell history. `READ_KEY_ID` is metadata, not the key secret.
`SESSION_COOKIE` is the cookie's `name=value` pair, not an API key. Do not enable
HTTP header/body tracing. Set the two non-secret origins, then run:

```sh
export API_URL=http://localhost:8088
export WEB_URL=http://localhost:3000
cargo run --quiet --manifest-path apps/api/Cargo.toml --example social_api
```

The example prints only a success summary, never credentials or response bodies.
If interrupted or a request fails, inspect the disposable account's posts and
key status before retrying; automatic recovery could duplicate an ambiguous write.

The exact example is run against real Rust listeners/PostgreSQL in this test:

```sh
python3 tests/with_postgres.py cargo test --manifest-path apps/api/Cargo.toml \
  --test gateway executable_documented_requests -- --ignored
```

That test uses an **upstream gateway identity fixture**, not hosted Unkey.
It tests application contracts and immediate local revocation even with a
replayed principal; it does not prove live provider verification or rate limits.
Credentialed real Frontline acceptance remains task 9.3. See
`apps/api/UNKEY.md` for provider setup and the browser-only key contract.
