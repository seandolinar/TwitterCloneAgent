# Tasks

All tasks are pending implementation. Follow `design.md` and the five capability specs; do not add deferred social features or deploy/provision services as part of this checklist. Use isolated test databases/Compose project names so tests never reset a developer's persistent volume.

## 1. Application and container foundation

- [ ] 1.1 Scaffold `apps/api/` with Axum/Tokio and `apps/web/` with TanStack Start/React/TypeScript; pin compatible toolchains/dependencies and commit lockfiles. Verify `cargo check --manifest-path apps/api/Cargo.toml` and `npm --prefix apps/web run typecheck`.
- [ ] 1.2 Add validated runtime configuration, private browser/gateway listeners, safe request IDs/error responses, body/time/concurrency bounds, and liveness endpoints. Verify configuration tests reject unsafe production settings and health requests contain no sensitive configuration.
- [ ] 1.3 Add multi-stage non-root application Dockerfiles, `.dockerignore` files, default Compose web/API/PostgreSQL/mail services, private networks, and a named database volume. Verify `docker compose config` and local application image builds without real secrets.
- [ ] 1.4 Add `.env.example`, secret/build-output ignore rules, and initial README setup commands. Verify the documented default startup requires no Unkey credentials and no host Rust or Node installation; inspect application image users and published ports.

## 2. Persistence and shared security infrastructure

- [ ] 2.1 Create versioned migrations for users, sessions, purpose-bound account tokens, posts, follows, likes, key metadata, rate-limit buckets, and delivery jobs, with the design's constraints/indexes. Verify migrations against a fresh isolated PostgreSQL database and rerun without destructive changes.
- [ ] 2.2 Add the one-shot migration command and database/schema readiness checks, wired into Compose startup. Verify backend readiness fails for an unavailable database or failed migration and succeeds only after schema initialization.
- [ ] 2.3 Implement atomic shared throttling, private identifier hashing, and expiration cleanup. Verify concurrent clients using separate backend instances share limits, receive 429/`Retry-After`, and cannot bypass limits when storage fails.
- [ ] 2.4 Implement bounded durable job claiming/retry for email intents and provider cleanup without persisting raw credentials. Verify crash/retry and concurrent-worker tests; document migration, throttle defaults, and job-recovery behavior alongside the implementation.

## 3. Account authentication and recovery

- [ ] 3.1 Implement normalized registration and uniqueness, the 15–128-code-point password policy, bounded Argon2id hashing, and uniform credential-failure behavior. Verify unit/integration tests for duplicate email/handle, invalid passwords, preserved password whitespace, and unknown-email login handling.
- [ ] 3.2 Implement email verification/resend and SMTP delivery with in-memory raw token generation and stored digests. Verify successful, expired, reused, wrong-purpose, concurrent-consumption, and failed-delivery cases, including a message delivered to the Compose mail catcher.
- [ ] 3.3 Implement opaque sessions, login rotation, logout, idle/absolute expiry, and recent password reauthentication. Verify old/revoked/expired tokens fail and production/development cookie attributes match the spec.
- [ ] 3.4 Implement CSRF bootstrap/token checks, exact-origin validation, verification restrictions, and auth endpoint throttles. Verify login-CSRF, missing-origin/token, hostile-origin, forged-identity, and unverified-user mutation attempts fail without side effects.
- [ ] 3.5 Implement password-reset request/consumption and atomic password replacement, session invalidation, local key revocation, and provider-cleanup enqueueing. Verify generic acknowledgements, reset races, expired/replayed tokens, and rejection of old sessions/keys immediately after commit.
- [ ] 3.6 Document the account HTTP contract, email/SMTP setup, recovery limitations, token lifetimes, and production security settings. Verify examples against the implemented endpoints and tests confirm no credential/email/token leakage in public responses or logs.

## 4. Social services and browser-facing API

- [ ] 4.1 Implement public profile DTOs and verified-owner edits for display name/biography, with fixed handles and relationship/post counts. Verify public privacy and cross-user edit rejection with PostgreSQL integration tests.
- [ ] 4.2 Implement text post create/read and author-only deletion with cascaded likes. Verify 0/1/280/281-code-point boundaries, whitespace-only/multibyte input, ignored or rejected spoofed author IDs, non-author deletion, and missing/deleted resources.
- [ ] 4.3 Implement follow/unfollow and like/unlike with unique constraints and idempotent HTTP behavior. Verify repeated/concurrent operations, self-follow/missing targets, counts, and both adapters' shared per-user write limit.
- [ ] 4.4 Implement profile posts and own-plus-followed chronological feeds with validated opaque cursors and 20-default/100-maximum page sizes. Verify timestamp ties, inserts between pages, empty feeds, unfollows, deleted posts, and invalid cursor/page-size errors.
- [ ] 4.5 Expose the services through the cookie-authenticated browser adapter and document its request/response/error shapes. Verify an authorization matrix covers anonymous, unverified, owner, and other-user requests without introducing key authentication on this listener.

## 5. Browser security boundary and account UI

- [ ] 5.1 Implement the TanStack Start fixed-origin, method/path-allowlisted proxy, request-scoped SSR data access, separate `Set-Cookie` forwarding, and sanitized identity/forwarding headers. Verify proxy tests block `/v1`, encoded traversal, arbitrary destinations, spoofed principals, and spoofed client IPs.
- [ ] 5.2 Add typed API access and request-scoped query/session state with no shared personalized cache, plus CSRF-token bootstrap/refresh. Verify two-user SSR/cache-isolation tests and browser tests preserve cookies and reject cross-origin mutations.
- [ ] 5.3 Build registration/login/logout, verification/resend, and forgotten/reset-password screens with safe generic acknowledgements. Verify Playwright can register, retrieve local email, verify, log in, reset, and observe invalidated old sessions.
- [ ] 5.4 Add responsive navigation, accessible form primitives, security headers, private-response cache controls, and safe plain-text rendering. Verify component tests, keyboard/mobile checks, hostile-text rendering, and no persistent password/session/API-key storage.
- [ ] 5.5 Document browser routes, local account onboarding, and verification/recovery troubleshooting. Verify the documented flow works from a clean browser using the default Compose stack.

## 6. Unkey management and third-party gateway

- [ ] 6.1 Implement a timeout-bounded Unkey REST management client using distinct creation API ID and verification keyspace ID. Verify HTTP contract tests cover non-recoverable keys, owner identity, allowed permissions, expiry, auto-applied limits, failures, and ambiguous create timeouts without blindly retrying.
- [ ] 6.2 Implement browser-only key create/list/revoke with recent-password checks, one-time secret responses, committed local metadata, local-first revocation, and durable provider cleanup. Verify read-only defaults, foreign-owner/permission injection, expired reauthentication, persistence failure, provider outage, and secret-redaction tests.
- [ ] 6.3 Configure a pinned real Frontline image and TOML route/policy in the optional `unkey` Compose profile, using a verify-only credential. Verify profile configuration rejects missing credentials clearly, leaves backend ports private, and never enables a verification bypass; document both required Unkey IDs and credential scopes.
- [ ] 6.4 Implement gateway-principal parsing and local key/account validation on the integration listener, enforce the documented permission matrix, and reuse the social services. Verify malformed/unknown principal versions, missing identity, wrong keyspace/owner, unknown/revoked/expired keys, cookie-only requests, and cross-user mutations fail.
- [ ] 6.5 Generate and publish the OpenAPI document from Rust DTOs with endpoint permissions and separate application/gateway errors. Verify contract coverage and executable example requests for post creation, cursor paging, read-only write rejection, and browser-only revocation.
- [ ] 6.6 Document Unkey propagation delay, local immediate denial, provider-cleanup retry, orphan-key reconciliation, and the production gateway trust boundary. Verify outage/recovery tests do not double-verify keys or double-count limits and application authorization never falls back to success.

## 7. Social and integration-key browser UI

- [ ] 7.1 Build the home feed, public profile, profile editor, and post-detail routes with paginated loading/empty/failure/end states. Verify browser tests for signed-out reads, own-plus-followed feeds, profile edits, and retry without duplicate posts.
- [ ] 7.2 Build the code-point-counted composer, owner-only deletion, follow controls, and like controls with server-confirmed or correctly rolled-back updates. Verify two-account browser tests cover all social actions, invalid text, count updates, and rejected mutations.
- [ ] 7.3 Build personal key settings with read-only defaults, permission/expiry controls, recent-password flow, transient one-time copy view, and revocation status. Verify UI tests show integration-unavailable states without credentials and never recover a full key after dismissal/navigation.
- [ ] 7.4 Finish keyboard/mobile interaction and screen-reader labels for social and key screens, and document a browser/API usage walkthrough. Verify accessible primary actions and that the walkthrough's examples match the published contract.

## 8. End-to-end acceptance and delivery evidence

- [ ] 8.1 Run Rust formatting/lint/unit/PostgreSQL integration checks and frontend type/component/production-build checks using documented scripts. Verify all capability scenarios have automated coverage or an explicit recorded manual check; report real commands and failures.
- [ ] 8.2 Run the exact default `docker compose up --build` workflow in an isolated project, then the Playwright acceptance suite against the resulting stack. Verify onboarding/recovery, cross-user ownership, social operations, proxy spoofing resistance, and session/cache isolation.
- [ ] 8.3 With user-supplied development credentials, run the exact `docker compose --profile unkey up --build` workflow and live smoke tests through real Frontline. Verify missing/invalid/wrong-keyspace/expired keys, permission denial, successful social automation, 429 behavior, and local immediate revocation. If credentials are absent, report this check as blocked, not passed.
- [ ] 8.4 Verify a normal container restart preserves accounts/posts/relationships/revocations, migrations are repeatable, database failure removes readiness, and inspected application images/assets contain no real secrets. Record the evidence without deleting persistent development volumes.
- [ ] 8.5 Review the final operational notes for HTTPS, production SMTP, private origin/gateway configuration, backups/restore, schema-compatible rollback, local-only services, and controlled-MVP limitations. Verify documentation does not claim production deployment or live-provider validation that was not performed.

## Workflow follow-up

- Present implementation and test evidence for review; do not publish images, provision paid services, merge a shared branch, or deploy without specific authorization.
- Sync/archive this change only when the user requests that separate OpenSpec workflow.
