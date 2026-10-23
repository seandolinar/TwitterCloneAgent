# Twitter MVP acceptance evidence

Validation date: 2026-10-23. Change: `build-twitter-mvp`.

## Credentialed local follow-up

After the user supplied development configuration, ran
`docker compose --profile unkey up --build --detach --wait --wait-timeout 120`
against the existing local project. Updated containers and real Frontline started
successfully; the existing database volume was preserved. Both profile
configuration/preflight tests passed. App, `/readyz` and Mailpit returned 200.

A private, temporary Python smoke driver exercised actual browser endpoints,
Mailpit SMTP, Unkey API Management and the real Frontline gateway. Passed:

- Missing key and invalid Bearer plus forged principal: gateway 401.
- New test account registration, email verification and login.
- Real one-time read-only key issuance through the browser management endpoint.
- Real gateway verification and `GET /v1/me` returning 200.
- Read-only key attempting post creation: 403.
- Bounded gateway reads exhausting the configured key limit: 429 with Retry-After.
- Browser revocation of the test key: 204; database check confirmed both local
  and provider revocation timestamps. No raw keys/passwords/email tokens were
  recorded in this report or test output.

The smoke driver was `python3 /private/tmp/twitterclone-live-smoke.py`; it is
ephemeral, not a committed reusable acceptance suite. Its dedicated local test
account remains; its sole provider key is revoked. The app/gateway were left
running for the user. This check establishes the exercised provider operations,
not a full permissions audit of the supplied root keys.

Task 9.3 remains unchecked: wrong-keyspace and provider-expired credentials,
successful integration writes, and immediate denial of an otherwise usable
key after local revocation were not exercised in this follow-up. Revocation
occurred after the key was rate-limited, so it cannot prove that last property.
No hosted Compute deployment or external production SMTP was performed.

The sections below record the earlier isolated acceptance pass; references to
missing credentials or unperformed provider checks describe that checkpoint.

This records local execution, not deployment or hosted-provider acceptance.
Docker 29.9.2 / Compose 5.6.1, Node 24.18.1, pinned Rust 1.99.1,
and pinned Playwright Chromium were used. Test credentials were generated only
for isolated fixtures; no developer database or `.env` was used.

## Commands and results

| Command (from repository root) | Result |
| --- | --- |
| `npm ci --prefix apps/web --offline --no-audit --no-fund` | Passed using the existing npm cache; lockfile unchanged. |
| `python3 tests/check_rust.py` | Passed locked Cargo check, formatting, all-target Clippy with warnings denied, 41 unit tests, and 55 PostgreSQL/HTTP tests (including six database-backed library tests). Disposable PostgreSQL removed. |
| `npm --prefix apps/web test` | 12 tests passed. |
| `npm --prefix apps/web run test:proxy` | Production build passed; its two HTTP tests initially hit sandbox socket restrictions. Re-running `node --test tests/proxy-runtime.ts tests/compute-runtime.ts` from `apps/web` with local socket access passed both tests against the same build. |
| `npm --prefix apps/web run typecheck` | Passed. |
| `python3 tests/test_foundation.py` | Four checks passed. |
| `python3 tests/test_unkey_profile.py` | Two configuration/preflight checks passed; no hosted verification performed. |
| `python3 tests/test_compute_config.py` | Three offline template checks passed. These do not establish acceptance by the provider's control plane. |
| `node --test tests/compute-supervisor.test.mjs` | Five process lifecycle checks passed. |
| `python3 tests/test_compute_runtime.py` | Passed real combined-image builds and local TLS PostgreSQL fixtures, restart/persistence, repeated migrations, local revocation, private listeners, ingress rejection, fixture environment isolation, TLS CA/hostname rejection, migration checksum failure, database-failure readiness, and sibling termination. Named fixture volumes preserved. |
| `python3 tests/test_foundation.py --live --acceptance` | Passed default foreground `docker compose up --build`, non-root web/API image checks, actual Mailpit SMTP delivery with digest-only token persistence, and all eight Chromium tests. Own containers/networks removed; named fixture volume preserved. |
| `docker compose --profile unkey up --build` and credentialed Frontline smoke tests | **Blocked:** no user-supplied development API ID, keyspace ID, management credential, or verify-only credential was available. Not run or passed. |
| Live Compute checklist | Not run; deployment/source upload/publication/provisioning were not authorized or performed. |

The first sandboxed frontend test run passed nine tests and failed three when
local HTTP fixtures could not bind (`EPERM`). The complete rerun with local
socket access passed all 12. The first uncached npm install could not finish
under restricted network access; installing locked dependencies from the existing
cache succeeded. The production build emitted upstream `use client` bundling
warnings; it completed successfully, and built-runtime tests passed.

The initial exact Compose acceptance run passed seven browser tests. Coverage
review added an eighth real-stack test for raw traversal, forged Bearer/principal
headers, and rotating caller-supplied IP headers. The complete exact Compose
workflow was rerun and all eight tests passed (including the original seven).
Five unknown-email login failures followed by 429/Retry-After demonstrated that
forged forwarding/private-IP headers cannot split the account/IP limit. Frontend
typecheck passed again after this addition. No runtime implementation changes
were needed during this validation pass.

Image inspection searched actual combined-image assets, the Rust binary, and
image metadata for generated fixture credentials. None were found. Build-context
checks exclude private environment/certificate files. No real credentials were
supplied, so this is fixture-secret inspection, not a claim to have scanned a
production image containing real secrets.

## Capability scenario coverage

Every scenario in the five delta specs is listed below. Rust paths are relative
to `apps/api/`; web paths are relative to `apps/web/`; other paths start at the
repository root. PostgreSQL tests exercise real SQL and HTTP adapters. Provider
fixtures and UI transport fixtures remain distinct from live Unkey acceptance.
Browser-suite execution is recorded separately above.

### account-authentication

| Scenario | Automated coverage |
| --- | --- |
| New registration | `tests/accounts.rs`, `tests/browser_auth.rs`, web `e2e/accounts.spec.ts` |
| Invalid or duplicate registration | `tests/accounts.rs`: uniqueness, preserved password whitespace, policy and transaction rollback |
| Verify an account | `tests/email.rs`: purpose-bound concurrent consumption; web `e2e/accounts.spec.ts` |
| Expired token or unverified mutation | `tests/email.rs`, `tests/browser_auth.rs`, `tests/browser_social.rs` |
| Successful login | `tests/sessions.rs`: rotation and cookie policy; web `e2e/accounts.spec.ts` |
| Logout or expiration | `tests/sessions.rs`: logout, idle and absolute expiry; web `e2e/accounts.spec.ts` |
| Reset a forgotten password | `tests/recovery.rs`, `tests/browser_auth.rs`, `tests/gateway.rs`: immediate cached-principal denial; web `e2e/accounts.spec.ts` |
| Invalid reset or email-delivery failure | `tests/recovery.rs`, `tests/email.rs`, web `e2e/accounts.spec.ts` |
| Cross-site mutation attempt | `tests/browser_auth.rs`, `tests/browser_social.rs`, web `e2e/security.spec.ts` |
| Repeated credential guessing | `tests/browser_auth.rs`, `tests/rate_limits.rs`: shared instances, normalized identifiers, 429/Retry-After |
| Private account information | `tests/accounts.rs`, `tests/social.rs`, `tests/browser_auth.rs`: public DTOs and real-process log redaction |

### social-timeline

| Scenario | Automated coverage |
| --- | --- |
| Read and update a profile | `tests/social.rs`, `tests/browser_social.rs`, web `e2e/social-scenarios.ts` via `e2e/security.spec.ts` |
| Reject another account's edit | `tests/social.rs`, `tests/browser_social.rs`, `tests/gateway.rs` |
| Publish a post | `tests/social.rs`, `tests/browser_social.rs`, web `e2e/social-scenarios.ts` |
| Reject invalid content | `tests/social.rs`: boundaries, whitespace, Unicode, spoofed author; web `tests/text.test.ts` and `e2e/social-scenarios.ts` |
| Author deletes a post | `tests/social.rs`: cascaded likes/counts; web `e2e/social-scenarios.ts` |
| Non-author deletion | `tests/social.rs`, `tests/browser_social.rs`, `tests/gateway.rs`, web `e2e/social-scenarios.ts` |
| Follow and unfollow | `tests/social.rs`, `tests/browser_social.rs`: concurrent/idempotent writes; web `e2e/social-scenarios.ts` |
| Invalid follow target | `tests/social.rs`, `tests/browser_social.rs` |
| Browse a home feed | `tests/social.rs`: current follows, own/empty feed; web `e2e/social-scenarios.ts` |
| Advance a cursor | `tests/social.rs`: timestamp ties, inserts, deletions, invalid cursors, default/max sizes; web `e2e/social-scenarios.ts` |
| Repeated likes and unlikes | `tests/social.rs`, `tests/browser_social.rs`, web `e2e/social-scenarios.ts` |
| Like a missing or deleted post | `tests/social.rs`, `tests/browser_social.rs` |

### developer-api

| Scenario | Automated coverage and live boundary |
| --- | --- |
| Publish through an integration | `tests/gateway.rs`, `tests/compute-client.mjs` through local trusted-ingress fixtures; hosted verification blocked under 9.3. |
| Attempt credential management through the API | `src/openapi.rs`, `tests/gateway.rs`, web `tests/compute-runtime.ts`: route/permission coverage and allowlists |
| Create a read-only key | `src/unkey.rs`, `src/keys/tests.rs`: exact provider contract and database-backed issuance with HTTP fixtures; real provider issuance blocked under 9.3. |
| Attempt to choose another owner or unknown permission | `src/unkey.rs`, `src/keys/tests.rs`, `tests/gateway.rs` |
| Revisit key settings | `src/keys/tests.rs`; web `e2e/key-scenarios.ts` uses explicit test transport responses for successful issuance. |
| Revoke during a provider outage | `src/keys/tests.rs`: local-first denial, unchanged upstream principal, durable cleanup retry, no Rust verification call |
| Reject invalid credentials | `tests/gateway.rs`: malformed, wrong-keyspace, expired/revoked/unknown key records; actual Frontline verification blocked under 9.3. |
| Forge a gateway identity | `tests/gateway.rs`, web `tests/proxy-runtime.ts`, `tests/compute-runtime.ts`, `e2e/security.spec.ts`, `tests/compute-client.mjs`; real gateway coverage remains separate. |
| Route browser and integration traffic through one ingress | Web `tests/compute-runtime.ts`, `tests/test_compute_config.py`, `tests/compute-client.mjs` |
| Use a read-only key to delete a post | `tests/gateway.rs`: permission intersection and cross-user ownership; hosted verification blocked under 9.3. |
| Exceed the configured request limit | `tests/rate_limits.rs`, `tests/gateway.rs`: shared user limits; `src/unkey.rs` checks auto-applied key-limit request fields. Live gateway 429/Retry-After blocked under 9.3. |
| Authentication dependency is unavailable | `src/unkey.rs`, `src/keys/tests.rs`, `tests/gateway.rs`, `tests/rate_limits.rs`: failures deny access; hosted outage behavior not exercised. |
| Integrate using the published contract | `src/openapi.rs`, `tests/gateway.rs::executable_documented_requests_match_the_application_contract` |

### web-client

| Scenario | Automated coverage |
| --- | --- |
| First-time account onboarding | `e2e/accounts.spec.ts`: actual SMTP through Compose Mailpit |
| Recover an expired verification or reset flow | `e2e/accounts.spec.ts`: invalid/reused-link UI; Rust `tests/email.rs` and `tests/recovery.rs`: actual expiry |
| Complete a social interaction | `e2e/social-scenarios.ts`: confirmed writes, ownership, counts and rejected-mutation state |
| Browse while signed out | `e2e/social-scenarios.ts`: public profiles/posts, missing resources, login invitation |
| Create and revoke a key | `e2e/key-scenarios.ts`: permission defaults, reauth, one-time reveal/copy/dismissal/navigation and revocation with explicit UI transport fixtures; live provider blocked under 9.3. |
| Render hostile text | `tests/security.test.ts`, `e2e/forms.spec.ts`, `e2e/social-scenarios.ts`: escaped posts/biographies and CSP |
| Switch browser sessions | `tests/api.test.ts`, `tests/proxy-runtime.ts`, `e2e/security.spec.ts`, `e2e/accounts.spec.ts` |
| Keyboard and narrow-screen use | `e2e/forms.spec.ts`, `e2e/social-scenarios.ts`, `e2e/key-scenarios.ts`: 320px viewport and labeled keyboard actions |
| Recover a failed page load | `e2e/social-scenarios.ts`: retained pages, retry, no duplicated IDs, end-of-list state |

### container-runtime

| Scenario | Automated coverage and operational boundary |
| --- | --- |
| Start a fresh checkout | `tests/test_foundation.py --live --acceptance`: default foreground Compose, schema, real SMTP and browser acceptance |
| Start a configured gateway | Configuration/preflight covered by `tests/test_unkey_profile.py`; credentialed manual acceptance blocked under 9.3, not passed. |
| Missing or invalid integration configuration | `tests/test_unkey_profile.py`, Rust `src/config.rs`; real provider credential rejection remains blocked. |
| Attempt direct backend access | `tests/test_foundation.py`: no published backend/database ports; built proxy tests and `tests/compute-client.mjs`: header/path rejection and private Rust sockets |
| Inspect a runtime image and configuration | `tests/test_foundation.py --live`, `tests/test_compute_config.py`, `tests/test_compute_runtime.py`: non-root users, allowlisted contexts, actual fixture-secret inspection |
| Start an unsafe production configuration | Rust `src/config.rs`, web `tests/runtime.test.ts`, `tests/test_compute_runtime.py`: production TLS/secret/origin/port restrictions |
| Restart the stack | `tests/test_compute_runtime.py` with `tests/compute-client.mjs`: accounts, posts, following, likes, and revoked-key denial after ordinary restarts; migrations rerun without reset |
| Database or migration failure | Rust `tests/database.rs`, `tests/test_compute_runtime.py`: unavailable database, missing/failed schema, corrupted migration checksum |
| Prepare a production deployment | `tests/test_compute_config.py` plus manual runbook review recorded below; no deployment performed. |
| Run the production image | `tests/test_compute_runtime.py`: actual combined image, single ingress, loopback Rust, external fixture dependencies |
| Startup or runtime dependency failure | `tests/compute-supervisor.test.mjs`, `tests/test_compute_runtime.py`: migration ordering, signals, sibling termination, TLS and readiness failures |
| Exercise a preview deployment | `tests/test_compute_runtime.py`: distinct fixture databases/credentials/keyspaces/origins and cross-environment denial only. Live deployment, real isolation and external SMTP require the separately authorized runbook checklist; not performed. |

## Operational review

Manual review completed for `README.md`, both application READMEs,
`apps/web/ACCOUNT_GUIDE.md`, `apps/api/UNKEY.md`, `docs/API.md`, and
`docs/deployment/unkey-compute.md`. Stale descriptions of planned integration,
pending key screens and acceptance were corrected. All 57 delta-spec scenarios
have a coverage entry above, with live-provider and deployment gaps identified.

The delivered Compose workflow and combined-image runbook describe different
topologies. Default Compose keeps Rust/PostgreSQL private and uses local Mailpit;
Compute requires the combined image, protected gateway ingress, external TLS
PostgreSQL and authenticated external SMTP. Runtime/gateway JSON files remain
operator templates, not submitted provider configuration.

Local fixture isolation establishes only separation of the fixture environments.
It does not establish separation of any hosted production/preview resources.
Provider limits, actual root-key scopes, real key verification, propagation,
external email deliverability, backup restoration and live rollback remain
operator validation. The runbook retains an unchecked live checklist requiring
separate authorization. No source upload, image publication, provisioning,
deployment, spec sync or archive was performed during this acceptance pass.
