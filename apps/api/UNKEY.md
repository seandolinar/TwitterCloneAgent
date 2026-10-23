# Unkey API Management client

The Rust management client, browser key API, and durable cleanup worker are
implemented and fixture-tested, along with private gateway principal/local-key
authorization. Key settings screens remain pending. No real provider keys were
issued during these tests.

## Separate the two Unkey IDs and credentials

- `UNKEY_API_ID` (`api_...`) selects the namespace in `keys.createKey`.
- `UNKEY_KEYSPACE_ID` (`ks_...`) is the verification/gateway identity namespace;
  it is **not** substituted for `apiId` on creation.
- `UNKEY_MANAGEMENT_ROOT_KEY` must be a private runtime credential scoped to
  create/delete keys in that API/keyspace. It must not have broad workspace
  administration or recoverable-key encryption grants.
- The gateway will use a separate **verify-only** credential; management and
  gateway verification are distinct products/operations. Rust does not call
  `keys.verifyKey` and must not double-count gateway verification or limits.
- `UNKEY_REQUESTS_PER_MINUTE` defaults to 60 and must be positive when management
  is configured. All replicas must share the intended configuration.

The default browser stack needs none of these credentials. Supply real values
only through private runtime configuration; never commit them, paste them into
chat, include them in frontend assets, or put them into fixture code.

## Creation contract

All management calls are POSTs to the fixed certificate-verified
`https://api.unkey.com` origin. There is no runtime URL override, proxy fallback,
redirect following, or automatic HTTP/protocol retry. Connect timeout is two
seconds; total request/body-read timeout five seconds; response bodies are
bounded to 16 KiB. The pinned Rustls ring provider is initialized explicitly and
shared with the existing database/mail TLS stack.

`POST /v2/keys.createKey` sends:

- The configured API ID and authenticated immutable user UUID as `externalId`.
  Unkey links/creates the identity automatically; no caller-selected owner is
  accepted by the application request DTO.
- A fixed `tc` prefix, 32 random bytes, explicit expiry, `enabled: true`, and
  **`recoverable: false`**.
- Only supported explicit social permissions: `profiles.read`,
  `profiles.write`, `posts.read`, `posts.write`, `feed.read`,
  `follows.write`, `likes.write`. No roles or wildcard permissions are requested.
- A named `requests` limit with configured positive count, `duration: 60000`
  milliseconds, and `autoApply: true`.

The application creation DTO accepts `name`, optional `permissions`, and
optional `expiresInDays`. Defaults are read-only (`profiles.read`, `posts.read`,
`feed.read`) and 90 days. Names are 1–100 code points, nonblank, with no control
characters; expiry is 1–365 days; empty/duplicate/unknown permissions are
rejected. Unknown fields, including `externalId`, owner IDs, `recoverable`, and
arbitrary provider options, are rejected.

The internal response contains only the provider key ID, exact expiry, and
transient secret. Secret-bearing types have no Debug/Serialize implementation;
their owned string and response buffer are zeroized on drop. Header diagnostics
mark Authorization as sensitive. A masked hint contains only a fixed prefix and
last four characters. Only the issuance service may expose the
secret once, **after local metadata commits**; it must never persist the secret.

## Failure and cleanup semantics

Create timeouts, transport failures, 5xx, malformed/oversized successes,
contradictory data/error envelopes, and invalid key identifiers/secrets produce
an **uncertain creation** result. A provider may already have created a key even
when its response was lost. Do not blindly retry: unknown local keys must deny
authorization, and provider records need deliberate orphan reconciliation.
The management error contains no raw provider response, credentials, or secret.
Other rejected requests are safe dependency failures, not successful issuance.

`POST /v2/keys.deleteKey` sends the key ID and `permanent: false`; it does not
irreversibly purge audit records. Cleanup is idempotent only for a valid empty
success or the provider's exact `unkey/data/key_not_found` error type. A generic
404 can mean wrong keyspace/root permissions and is **not** treated as deletion.
Provider cleanup is durably retried by the dedicated queue driver.
Application-local revocation remains authoritative after commit, including while
provider propagation/cleanup is pending.

## Validation and reference contract

The unit tests start isolated local HTTP servers reachable only inside tests.
They exercise the real client serializer/parser, owner/API-ID/permission/expiry
fields, read-only defaults, non-recoverability, auto-applied limits, invalid
input, timeout/no-retry behavior, malformed/oversized/error responses, and
idempotent soft deletion. They are not an application runtime mock or a live
Unkey connectivity/authorization check.

```sh
cargo test --manifest-path apps/api/Cargo.toml unkey::tests
```

Provider contracts consulted:

- [Creating keys](https://www.unkey.com/docs/api-management/keys/creating-keys)
- [Create API key reference](https://www.unkey.com/docs/platform/api-reference/keys/create-api-key)
- [Delete API keys reference](https://www.unkey.com/docs/platform/api-reference/keys/delete-api-keys)

Live Frontline/API Management/Compute evidence remains separate and requires the
appropriate user-supplied credentials and, for deployment/publication, separate
authorization.

## Private gateway application authorization

Only `/v1` social routes exist on the integration listener. Account/credential
and key-management endpoints do not exist there. Browser cookies and raw Bearer
values cannot substitute for a trusted `X-Unkey-Principal`.

The header is bounded JSON, exactly one value, with `version: "v1"`,
`type: "API_KEY"`, a UUID `identity.externalId`, a matching subject, and
`source.key.keyId`/`keySpaceId`. A missing identity never falls back to the
subject. Unknown optional fields are ignored for forward compatibility.
Malformed/unknown versions, wrong types/keyspaces/owners, missing fields, and
duplicate headers fail closed.

Every operation freshly queries the local key/account row: registered key,
matching owner/keyspace, verified account, unexpired key, and no local
revocation. An optional provider expiry cannot extend the local lifetime.
Database failure returns 503, not anonymous or authorized fallback. Operation
permissions are the intersection of gateway-granted and locally stored slugs;
roles/wildcards do not confer extra application permissions.

| Operations | Required permission |
| --- | --- |
| `GET /v1/me`, `GET /v1/profiles/{handle}` | `profiles.read` |
| `PATCH /v1/me` | `profiles.write` |
| `GET /v1/profiles/{handle}/posts`, `GET /v1/posts/{id}` | `posts.read` |
| `GET /v1/feed` | `feed.read` |
| `POST /v1/posts`, `DELETE /v1/posts/{id}` | `posts.write` |
| `PUT`/`DELETE /v1/me/following/{id}` | `follows.write` |
| `PUT`/`DELETE /v1/posts/{id}/like` | `likes.write` |

Both adapters call the same social handlers/services for input validation,
verified ownership, counts, pagination, and shared per-user writes. Missing
permission returns 403 before a mutation; a key with write scope still cannot
delete another author's post or choose a different owner. New requests after
reset/revocation commit are denied even if the provider replays cached identity.
Application requests already in flight may finish.

These checks do **not** cryptographically sign the header. A valid-looking
principal can be forged by anyone who can directly reach the private listener:
its network/proxy isolation is mandatory. The ordinary browser proxy strips
principals, and real Frontline rejects a caller principal without a key. Direct
application integration tests synthesize trusted gateway headers as fixtures;
they are not evidence of real provider key verification.

```sh
python3 tests/with_postgres.py cargo test \
  --manifest-path apps/api/Cargo.toml --test gateway -- --ignored
```

## Optional local Frontline profile

The optional Compose profile uses the real multi-architecture
`ghcr.io/unkeyed/frontline:v1.0.26-rc.1` image, pinned to OCI index digest
`sha256:86f286665f6756d3f34b42bc44a44c4c83772f9e14ef7ba5c879deb6403d03bf`.
Its versioned TOML is `infra/unkey/gateway.toml`, following Unkey's documented
local-dev contract. It is **not** a self-contained Unkey control plane or a
production Compute configuration.

Before selecting the profile, supply all four private/configuration inputs:

| Input | Purpose/scope |
| --- | --- |
| `UNKEY_MANAGEMENT_ROOT_KEY` | Create/delete keys for the chosen API/keyspace only; no encrypt or workspace administration |
| `UNKEY_API_ID` | `api_...` creation API ID from that keyspace's settings |
| `UNKEY_KEYSPACE_ID` | Distinct `ks_...` verification ID for the same keyspace |
| `UNKEY_VERIFY_ROOT_KEY` | Separate verify-only root credential, e.g. the narrow `api.<api_id>.verify_key` grant; no create/delete grants |

The operator must verify actual provider credential scopes; local syntax checks
cannot prove what permissions a root key has. Management and verification
credentials must be distinct. Both credentials and IDs must remain specific to
this environment, not shared with production.

```sh
docker compose --profile unkey up --build
```

The dedicated integration gateway is http://localhost:8088 by default;
`UNKEY_GATEWAY_PORT` changes its loopback port independently of the browser
origin. Browser/mail addresses remain unchanged. Frontline has only the gateway
network and reaches `api:8081`, never the browser listener. Neither Rust listener
nor PostgreSQL publishes a host port. The browser proxy still blocks `/v1` and
strips caller principals; only the dedicated Frontline entry can supply trusted
integration identity.

The profile-only `unkey-preflight` runs before Frontline, without a network,
and rejects missing/unsafe IDs/credentials, swapped API/keyspace IDs, and
accidental root-key reuse with non-secret setup messages. Keeping these checks
out of required Compose interpolation allows `docker compose up --build` to
work with all Unkey inputs empty. Preflight is not a verification bypass.

Every gateway route is guarded by the TOML key-auth policy, which uses only the
verify credential and configured keyspace. Bearer credentials are required; a
cookie or caller-supplied principal cannot replace them. Created keys carry the
auto-applied request limit, verified once at Frontline. Rust separately checks
the principal and current local account/key authorization on every request;
provider acceptance alone cannot authorize an unknown/revoked local key.

Checks:

```sh
python3 tests/test_unkey_profile.py
python3 tests/test_unkey_profile.py --live
```

The first checks profile/network/credential structure and exact preflight
behavior. `--live` runs the pinned real Frontline image with isolated fixture
configuration and **no Bearer key**: it verifies native TOML loading, rejection
of missing keys/forged principals, and actual Docker preflight failure for a
missing verify credential. It does not prove live provider authentication,
valid-key forwarding, provider outages/rate limits, or real credential scopes.
Those checks remain task 9.3 and are blocked until user-supplied scoped
credentials and real disposable keys are available. Its containers/networks are
removed afterward without deleting its named database volume.

Reference:
[Local gateway contract](https://www.unkey.com/docs/compute/gateway/local-development).

## Browser key HTTP contract

These routes exist only on the cookie listener. The frontend allowlist maps
`/api/auth/keys` to `/browser/auth/keys`; no integration key can create another
key or manage credentials.

| Method and private path | Authorization | Success |
| --- | --- | --- |
| `GET /browser/auth/keys` | Live verified session | 200 `{ "integrationAvailable": true, "items": [<metadata>] }` |
| `POST /browser/auth/keys` | Live verified session, password within five minutes, original Origin/CSRF | 201 `{ "key": "<one-time-secret>", "metadata": <metadata> }` |
| `DELETE /browser/auth/keys/{key_id}` | Live verified owner, original Origin/CSRF | Empty 204; idempotent for an owned revoked key |

Creation accepts `{ "name": "Automation", "permissions": ["profiles.read",
"posts.read", "feed.read"], "expiresInDays": 90 }`. Omitting permissions/expiry
uses those defaults. `POST /browser/auth/reauthenticate` with the current password
refreshes the five-minute window. Service authorization rechecks the live
database session and verification state, not a cached session boolean.

Metadata contains `keyId`, `name`, `maskedHint`, `permissions`, `createdAt`,
`expiresAt`, `revokedAt`, `providerRevokedAt`, and `status`. Status is `active`,
`expired`, `revoked_pending_cleanup`, or `revoked`. Lists contain neither full
keys nor another account's records. Creation is the sole secret-bearing response;
all responses are no-store. Missing/foreign keys return the same 404. Missing
recent password authentication returns 403 `recent_password_required`.

Without provider configuration, verified users can still list committed
metadata with `integrationAvailable: false`; creation returns 503
`integration_unavailable`, never a fake key. Local revocation still works and
queues provider cleanup for when configuration is restored. Key-management
operations share a PostgreSQL limit of 20 attempts per minute per account.

### Transaction and worker boundary

Issuance locks the account before freshly checking its session, calling the
provider, and committing local metadata. Password reset uses the same account
lock, so it either denies a reset-revoked session before issuance or revokes
the newly committed key afterward. The raw secret is returned only after a
successful local commit.

Persistence/commit failure never returns a secret. A fresh transaction locally
revokes any uncertain committed record and queues cleanup of the known provider
ID—even when no local metadata row exists. If storage is completely unavailable,
it cannot persist an intent; it attempts provider deletion, and an ambiguous
outcome remains a manual orphan-reconciliation case. A lost response or cancelled
in-flight provider creation can likewise require reconciliation; neither case
authorizes an unknown key or justifies blindly replaying create.

Revocation commits locally with its cleanup intent, before any provider call.
The cleanup worker claims only `unkey_revoke` jobs, leaving email delivery to
the separate worker. It uses existing bounded leases/retry budgets; successful
soft deletion marks `providerRevokedAt`, then acknowledges the claim. Provider
failure leaves the key locally revoked and reschedules cleanup. Missing
configuration does not drain or falsely acknowledge cleanup intents.

```sh
python3 tests/with_postgres.py cargo test \
  --manifest-path apps/api/Cargo.toml keys::tests -- --ignored
```

PostgreSQL/HTTP fixture tests cover owner isolation, one-time/no-store secret
responses, non-secret persistence/listing, injection rejection, stale
reauthentication, verification/revoked-session checks, local-first idempotent
revocation, provider-outage retry/recovery, failed persistence cleanup, and
reset/issuance serialization. These tests do not establish live gateway denial
or live provider connectivity; those remain separately tracked.
