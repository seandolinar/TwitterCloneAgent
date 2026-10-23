# Proposal

## Why

The project has no application yet. Build an elementary Twitter-style product that people can use in a browser or through user-owned API keys, with a Rust backend, secure account authentication, and a reproducible Docker development environment.

## What Changes

- Add a Rust/Axum backend and PostgreSQL persistence, with application logic and authorization owned by Rust.
- Add a React/TypeScript frontend using TanStack Start.
- Support registration, email verification, login/logout, password reset, and revocable cookie sessions.
- Support public profiles, text-only posts up to 280 characters, following, a chronological home feed, likes, and deleting one's own posts.
- Expose a documented, versioned third-party API for the same social operations, using scoped user-owned keys issued through Unkey API Management and authenticated by Unkey's gateway.
- Provide browser-only API-key creation, listing, and revocation. API keys cannot manage account credentials or mint other keys.
- Dockerize the frontend and backend, and provide Compose services for PostgreSQL, local email capture, and Unkey Frontline integration.
- Target Unkey Compute for production with one combined frontend/backend container, a single gateway-facing ingress, and loopback-only Rust listeners. Include the production image, deployment configuration, and an operator runbook; actual provisioning, publication, and deployment require separate authorization.
- Keep replies, reposts, media, DMs, search, notifications, OAuth/social login, MFA, and moderation/admin tools out of this MVP. An unmoderated MVP is not ready for an unrestricted public launch.

## Capabilities

### New Capabilities

- `account-authentication`: Email/password registration, email verification and recovery, protected sessions, and authentication abuse defenses.
- `social-timeline`: Profiles, bounded text posts, owner-only deletion, following, chronological feeds, and likes.
- `developer-api`: A versioned user-scoped API, Unkey key lifecycle and gateway authentication, permissions, rate limits, and API documentation.
- `web-client`: TanStack Start screens and browser workflows for authentication, social interactions, and API-key management.
- `container-runtime`: Reproducible Docker builds, Compose startup, migrations, health checks, safe runtime configuration, and combined-container delivery for Unkey Compute.

### Modified Capabilities

None. There are no existing application specifications.

## Impact

- Affected code areas: `apps/api/`, `apps/web/`, `infra/`, `docs/`, and `tests/`. Application/container/database foundations now exist; the task checklist records the remaining work.
- New dependencies: Rust/Axum/Tokio, SQLx/PostgreSQL, TanStack Start/React/TypeScript, maintained password-hashing and email libraries, and Docker Compose.
- External dependencies: an Unkey workspace/keyspace and restricted server credentials; an SMTP provider for production verification/reset email. Local email uses a mail catcher.
- Unkey API Management and its gateway are distinct integrations. The documented Frontline local-development mode verifies real keys online; it is not a self-contained offline Unkey installation or a production deployment.
- Production hosting is Unkey Compute. PostgreSQL is externally hosted with authenticated, certificate-verified TLS, and production email uses external authenticated SMTP. Production and preview use separate databases/data, Unkey keyspaces, runtime credentials, and origins.
- The combined app avoids public service-to-service calls because Compute does not provide private app-to-app networking. Frontend/backend releases and scaling are coupled for this MVP.
- Domains, PostgreSQL/SMTP vendors, resource sizing, and paid service provisioning remain operator choices. Planning and implementation do not authorize deployment, publishing images, or uploading private source to a hosting provider.
