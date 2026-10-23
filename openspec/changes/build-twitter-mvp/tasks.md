# Tasks

Checked tasks are implemented; unchecked tasks remain pending. Follow `design.md` and the five capability specs. This checklist is authoritative for the consolidated work in this thread: preserve completed tasks rather than importing an older draft's checkbox state. Do not add deferred social features or deploy/provision services. Use isolated test databases/Compose project names so tests never reset a developer's persistent volume.

## 1. Application and container foundation

- [x] 1.1 Scaffold `apps/api/` with Axum/Tokio and `apps/web/` with TanStack Start/React/TypeScript; pin compatible toolchains/dependencies and commit lockfiles. Verify `cargo check --manifest-path apps/api/Cargo.toml` and `npm --prefix apps/web run typecheck`.
- [x] 1.2 Add validated runtime configuration, private browser/gateway listeners, safe request IDs/error responses, body/time/concurrency bounds, and liveness endpoints. Verify configuration tests reject unsafe production settings and health requests contain no sensitive configuration.
- [x] 1.3 Add multi-stage non-root application Dockerfiles, `.dockerignore` files, default Compose web/API/PostgreSQL/mail services, private networks, and a named database volume. Verify `docker compose config` and local application image builds without real secrets.
- [x] 1.4 Add `.env.example`, secret/build-output ignore rules, and initial README setup commands. Verify the documented default startup requires no Unkey credentials and no host Rust or Node installation; inspect application image users and published ports.

## 2. Persistence and shared security infrastructure

- [x] 2.1 Create versioned migrations for users, sessions, purpose-bound account tokens, posts, follows, likes, key metadata, rate-limit buckets, and delivery jobs, with the design's constraints/indexes. Verify migrations against a fresh isolated PostgreSQL database and rerun without destructive changes.
- [x] 2.2 Add the one-shot migration command and database/schema readiness checks, wired into Compose startup. Verify backend readiness fails for an unavailable database or failed migration and succeeds only after schema initialization.
- [x] 2.3 Implement atomic shared throttling, private identifier hashing, and expiration cleanup. Verify concurrent clients using separate backend instances share limits, receive 429/`Retry-After`, and cannot bypass limits when storage fails.
- [x] 2.4 Implement bounded durable job claiming/retry for email intents and provider cleanup without persisting raw credentials. Verify crash/retry and concurrent-worker tests; document migration, throttle defaults, and job-recovery behavior alongside the implementation.

## 3. Account authentication and recovery

- [x] 3.1 Implement normalized registration and uniqueness, the 15–128-code-point password policy, bounded Argon2id hashing, and uniform credential-failure behavior. Verify unit/integration tests for duplicate email/handle, invalid passwords, preserved password whitespace, and unknown-email login handling.
- [x] 3.2 Implement email verification/resend and SMTP delivery with in-memory raw token generation and stored digests. Verify successful, expired, reused, wrong-purpose, concurrent-consumption, and failed-delivery cases, including a message delivered to the Compose mail catcher.
- [x] 3.3 Implement opaque sessions, login rotation, logout, idle/absolute expiry, and recent password reauthentication. Verify old/revoked/expired tokens fail and production/development cookie attributes match the spec.
- [x] 3.4 Implement CSRF bootstrap/token checks, exact-origin validation, verification restrictions, and auth endpoint throttles. Verify login-CSRF, missing-origin/token, hostile-origin, forged-identity, and unverified-user mutation attempts fail without side effects.
- [x] 3.5 Implement password-reset request/consumption and atomic password replacement, session invalidation, local key revocation, and provider-cleanup enqueueing. Verify generic acknowledgements, reset races, expired/replayed tokens, and rejection of old sessions/keys immediately after commit.
- [x] 3.6 Document the account HTTP contract, email/SMTP setup, recovery limitations, token lifetimes, and production security settings. Verify examples against the implemented endpoints and tests confirm no credential/email/token leakage in public responses or logs.

## 4. Social services and browser-facing API

- [x] 4.1 Implement public profile DTOs and verified-owner edits for display name/biography, with fixed handles and relationship/post counts. Verify public privacy and cross-user edit rejection with PostgreSQL integration tests.
- [x] 4.2 Implement text post create/read and author-only deletion with cascaded likes. Verify 0/1/280/281-code-point boundaries, whitespace-only/multibyte input, ignored or rejected spoofed author IDs, non-author deletion, and missing/deleted resources.
- [x] 4.3 Implement follow/unfollow and like/unlike with unique constraints and idempotent HTTP behavior. Verify repeated/concurrent operations, self-follow/missing targets, counts, and both adapters' shared per-user write limit.
- [x] 4.4 Implement profile posts and own-plus-followed chronological feeds with validated opaque cursors and 20-default/100-maximum page sizes. Verify timestamp ties, inserts between pages, empty feeds, unfollows, deleted posts, and invalid cursor/page-size errors.
- [x] 4.5 Expose the services through the cookie-authenticated browser adapter and document its request/response/error shapes. Verify an authorization matrix covers anonymous, unverified, owner, and other-user requests without introducing key authentication on this listener.

## 5. Browser security boundary and account UI

- [x] 5.1 Implement the TanStack Start fixed-origin, method/path-allowlisted browser proxy, request-scoped SSR data access, separate `Set-Cookie` forwarding, and sanitized identity/forwarding headers. Verify the standard local browser entry blocks `/v1`, encoded traversal, arbitrary destinations, spoofed principals, and spoofed client IPs; the Compute gateway branch is tracked separately in 8.2.
- [x] 5.2 Add typed API access and request-scoped query/session state with no shared personalized cache, plus CSRF-token bootstrap/refresh. Verify two-user SSR/cache-isolation tests and browser tests preserve cookies and reject cross-origin mutations.
- [x] 5.3 Build registration/login/logout, verification/resend, and forgotten/reset-password screens with safe generic acknowledgements. Verify Playwright can register, retrieve local email, verify, log in, reset, and observe invalidated old sessions.
- [x] 5.4 Add responsive navigation, accessible form primitives, security headers, private-response cache controls, and safe plain-text rendering. Verify component tests, keyboard/mobile checks, hostile-text rendering, and no persistent password/session/API-key storage.
- [x] 5.5 Document browser routes, local account onboarding, and verification/recovery troubleshooting. Verify the documented flow works from a clean browser using the default Compose stack.

## 6. Unkey management and third-party gateway

- [x] 6.1 Implement a timeout-bounded Unkey REST management client using distinct creation API ID and verification keyspace ID. Verify HTTP contract tests cover non-recoverable keys, owner identity, allowed permissions, expiry, auto-applied limits, failures, and ambiguous create timeouts without blindly retrying.
- [x] 6.2 Implement browser-only key create/list/revoke with recent-password checks, one-time secret responses, committed local metadata, local-first revocation, and durable provider cleanup. Verify read-only defaults, foreign-owner/permission injection, expired reauthentication, persistence failure, provider outage, and secret-redaction tests.
- [x] 6.3 Configure a pinned real Frontline image and TOML route/policy in the optional `unkey` Compose profile, using a verify-only credential. Verify profile configuration rejects missing credentials clearly, leaves backend ports private, and never enables a verification bypass; document both required Unkey IDs and credential scopes.
- [x] 6.4 Implement gateway-principal parsing and local key/account validation on the integration listener, enforce the documented permission matrix, and reuse the social services. Verify malformed/unknown principal versions, missing identity, wrong keyspace/owner, unknown/revoked/expired keys, cookie-only requests, and cross-user mutations fail.
- [ ] 6.5 Generate and publish the OpenAPI document from Rust DTOs with endpoint permissions and separate application/gateway errors. Verify contract coverage and executable example requests for post creation, cursor paging, read-only write rejection, and browser-only revocation.
- [ ] 6.6 Document Unkey propagation delay, local immediate denial, provider-cleanup retry, orphan-key reconciliation, and the production gateway trust boundary. Verify outage/recovery tests do not double-verify keys or double-count limits and application authorization never falls back to success.

## 7. Social and integration-key browser UI

- [ ] 7.1 Build the home feed, public profile, profile editor, and post-detail routes with paginated loading/empty/failure/end states. Verify browser tests for signed-out reads, own-plus-followed feeds, profile edits, and retry without duplicate posts.
- [ ] 7.2 Build the code-point-counted composer, owner-only deletion, follow controls, and like controls with server-confirmed or correctly rolled-back updates. Verify two-account browser tests cover all social actions, invalid text, count updates, and rejected mutations.
- [ ] 7.3 Build personal key settings with read-only defaults, permission/expiry controls, recent-password flow, transient one-time copy view, and revocation status. Verify UI tests show integration-unavailable states without credentials and never recover a full key after dismissal/navigation.
- [ ] 7.4 Finish keyboard/mobile interaction and screen-reader labels for social and key screens, and document a browser/API usage walkthrough. Verify accessible primary actions and that the walkthrough's examples match the published contract.

## 8. Unkey Compute delivery

- [ ] 8.1 Add the combined non-root production image and supervised entrypoint for TanStack Start and Rust, with one-shot migrations before serving. Verify local image builds, SIGTERM propagation, child-failure termination, migration failure, and no development database/mail service or embedded secrets.
- [ ] 8.2 Add the explicit Compute-mode single ingress: browser routes proxy to the loopback cookie interface and allowlisted `/v1` operations to the loopback gateway interface. Verify route normalization, principal/header preservation only on the integration branch, stripped browser identity headers, and no cross-mode authentication fallback.
- [ ] 8.3 Validate production `PORT`, loopback bindings, exact HTTPS origin, certificate-verified PostgreSQL TLS, external authenticated SMTP, and required environment-specific secrets. Verify invalid/partial configurations fail safely and readiness covers both processes and schema without exposing configuration.
- [ ] 8.4 Add versioned Compute runtime/gateway configuration for the combined image, `/readyz` health checks, and `/v1`-only key authentication/rate limits. Verify configuration against the current provider contract and tests that browser/health routes remain outside key-auth policy while every integration route is covered.
- [ ] 8.5 Write `docs/deployment/unkey-compute.md` covering private-source Dockerfile builds or separately authorized image publication, runtime secret injection, external PostgreSQL/SMTP, production/preview isolation, migrations, resource sizing, backups, and schema-compatible rollback. Verify the runbook and examples match delivered configuration; do not provision, upload source, publish images, or deploy.
- [ ] 8.6 Add local combined-topology smoke/security tests using isolated dependency fixtures, and a separately authorized live Compute smoke-test checklist. Verify single-port ingress, private Rust interfaces, path/header bypass rejection, failure readiness, and production/preview isolation assertions; report fixture validation separately from any live deployment evidence.

## 9. End-to-end acceptance and delivery evidence

- [ ] 9.1 Run Rust formatting/lint/unit/PostgreSQL integration checks and frontend type/component/production-build checks using documented scripts. Verify all capability scenarios have automated coverage or an explicit recorded manual check; report real commands and failures.
- [ ] 9.2 Run the exact default `docker compose up --build` workflow in an isolated project, then the Playwright acceptance suite against the resulting stack. Verify onboarding/recovery, cross-user ownership, social operations, proxy spoofing resistance, and session/cache isolation.
- [ ] 9.3 With user-supplied development credentials, run the exact `docker compose --profile unkey up --build` workflow and live smoke tests through real Frontline. Verify missing/invalid/wrong-keyspace/expired keys, permission denial, successful social automation, 429 behavior, and local immediate revocation. If credentials are absent, report this check as blocked, not passed.
- [ ] 9.4 Verify a normal container restart preserves accounts/posts/relationships/revocations, migrations are repeatable, database failure removes readiness, and inspected application images/assets contain no real secrets. Record the evidence without deleting persistent development volumes.
- [ ] 9.5 Review the final operational notes and both local/combined-image evidence, including the live Compute checklist if separately authorized. Verify documentation does not claim production deployment, environment isolation, or live-provider checks that were not performed.

## Workflow follow-up

- Present implementation and test evidence for review; do not publish images, provision paid services, merge a shared branch, or deploy without specific authorization.
- Sync/archive this change only when the user requests that separate OpenSpec workflow.

## Current implementation evidence

- Task 3.4 is implemented with private browser account routes, session-bound signed CSRF pairs, exact-origin validation, verified-user guards, and configurable shared authentication throttles.
- Tasks 3.5 and 4.3 are now closed by request-level gateway tests: an unchanged cached principal fails on new reads/writes immediately after reset, and browser/gateway mutations consume one shared per-user budget. Transaction, expiry/replay, reset/issuance races, idempotency, ownership, and rollback tests remain in place. Local state assertions alone were not used to close these acceptance checks.
- Tasks 6.1–6.4 provide bounded non-recoverable key management, browser-only one-time issuance, local-first revocation/durable cleanup, a pinned real Frontline profile, and private principal/local-key/permission authorization through the shared social services. Provider contract/outage tests use isolated fixtures; the real Frontline image check proves native config loading and missing-key/forged-header denial, not real provider credential acceptance.
- Backend checkpoint checks pass 37 unit tests and all 54 PostgreSQL/HTTP integration tests, including five gateway request tests, plus Clippy with `--all-targets -- -D warnings` and formatting in the pinned `rust:1.99.1-bookworm` container. The host lacks Rust; the PostgreSQL wrapper passes its isolated database through `host.docker.internal`, with build outputs inside the container.
- Tasks 5.1–5.5 now provide the browser proxy, request-scoped Query/session state, account UI, accessible form/navigation primitives, production nonce CSP, and the account guide. Frontend validation passed 10 unit/component tests, typecheck, the production build/built-Nitro proxy/two-user SSR checks, and seven real Chromium tests against isolated Compose web/Rust/PostgreSQL/Mailpit. Those tests cover onboarding/recovery, old-session invalidation, resend/invalid/reused links, HttpOnly cookies, cross-origin/CSRF denial, SSR separation, mobile/keyboard/forms/storage, and CSP/hydration.
- The separately requested preview was smoke-tested and then torn down at the user's request, preserving its named database volume. Browser test stacks also tear down their own containers/networks without deleting named volumes. Full-product social/key browser acceptance, live Unkey, and Compute delivery/deployment checks remain pending; do not mark 9.x complete from these partial-product results.
