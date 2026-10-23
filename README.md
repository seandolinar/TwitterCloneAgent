# TwitterCloneAgent

A text-first Twitter-style MVP with a Rust/Axum API, a TanStack Start frontend,
PostgreSQL, and planned Unkey API-key/gateway integration.

**Current status:** container/database foundations, browser account screens and
secure sessions/recovery, the social API (profiles, posts, relationships, and
timelines), and the allowlisted browser proxy are implemented. See the
[account walkthrough](apps/web/ACCOUNT_GUIDE.md) to try the usable browser flows.
Feed/profile/social screens, key settings UI, and live Unkey acceptance remain
pending. Home is an account-aware landing
page, not a finished social network. Do not deploy it as a finished product.

Browser key-management endpoints and the optional pinned Frontline Compose
profile and gateway principal/local-key authorization are implemented;
see [the Unkey contract](apps/api/UNKEY.md). Provider-authenticated live
acceptance remains separate from local fixture evidence.

**Approved production target:** one Unkey Compute app containing TanStack Start
and Rust behind a single ingress, with loopback-only Rust interfaces and external
TLS PostgreSQL/SMTP. The combined image, deployment configuration, and runbook are
still pending. Local development continues to use the Compose stack below.
The plan and implementation progress live in
[`build-twitter-mvp`](openspec/changes/build-twitter-mvp/tasks.md); earlier draft
threads do not replace its checked task state. Deployment and provisioning need
separate authorization.

## Run with Docker

Requires a running Docker engine and Docker Compose. No host Rust or Node
installation is needed to build and run the application containers.

1. Copy the non-secret template:

   ```sh
   cp .env.example .env
   ```

2. Run `openssl rand -hex 32` **three separate times** in your local terminal and
   save the independent outputs in `.env` as `POSTGRES_PASSWORD`, `CSRF_SECRET`,
   and `RATE_LIMIT_SECRET`. Hexadecimal output is URL-safe for the PostgreSQL
   connection URL. Never commit `.env` or paste its values into chat.
3. Leave the Unkey settings empty for the default stack.
4. Start the stack:

   ```sh
   docker compose up --build
   ```

- Web: http://localhost:3000
- Local mail UI: http://localhost:8025
- API and PostgreSQL: private Docker networks, **no published host ports**.
- `WEB_PORT` and `MAIL_UI_PORT` can change loopback ports. If changing the web
  port, also update `APP_ORIGIN` to the exact external browser origin.

Both security secrets are required in **development and production**. The backend
rejects missing, blank, shorter-than-32-byte, or identical values. These are local
application secrets, not Unkey credentials. Keep them stable across restarts and
use the same configuration for every backend instance; do not regenerate them
per process. Compose checks that they are present, and the backend checks length
and distinctness. Neither layer supplies a default or generates a fallback.

Stop with Ctrl+C, or run `docker compose down`. PostgreSQL uses a named volume;
ordinary stops/restarts preserve it. **`docker compose down -v` deletes local
database data** and is not part of normal startup or validation.

The mail catcher is for local development only. Compose waits for PostgreSQL,
runs the one-shot `migrate` service, and starts the API only after successful
initialization. API `/healthz` checks process liveness; `/readyz` checks database
connectivity, migration versions/checksums, and required tables. The API container
health check uses readiness. Unkey is not part of browser-stack readiness.

## Development checks

Rust is pinned by `rust-toolchain.toml`; Node is pinned by `.nvmrc`. Commit both
application lockfiles. Do not substitute floating dependency versions.

```sh
cargo check --manifest-path apps/api/Cargo.toml
cargo test --manifest-path apps/api/Cargo.toml
cargo fmt --manifest-path apps/api/Cargo.toml --check
cargo clippy --manifest-path apps/api/Cargo.toml --all-targets -- -D warnings

npm ci --prefix apps/web
npm --prefix apps/web run build
npm --prefix apps/web run typecheck
```

With Python 3 and Docker available, run the foundation configuration checks:

```sh
python3 tests/test_foundation.py
```

Run PostgreSQL constraint/migration/readiness and shared-throttling tests in a
disposable container:

```sh
python3 tests/with_postgres.py cargo test \
  --manifest-path apps/api/Cargo.toml --tests -- --ignored
```

To build/start an isolated Compose project, check the running frontend/mail/API
containers, and inspect non-root image users:

```sh
python3 tests/test_foundation.py --live
```

This test uses ephemeral credentials and free loopback ports, not your `.env`
credentials. It also delivers a real verification-email fixture through SMTP to
the Compose mail catcher and checks digest-only token persistence without
printing the token. It stops its own containers without deleting named volumes.
It does not test complete browser authentication/social workflows or live Unkey.

See [the API README](apps/api/README.md) for configuration validation, HTTP
limits, and backend behavior. Production requires HTTPS, private ingress,
proper secrets, external authenticated SMTP, backups, and the remaining
implementation/security work—not just a change to `APP_ENV`.

## Specification-driven development

This project uses [OpenSpec](https://github.com/Fission-AI/OpenSpec) to propose,
implement, and archive changes.

The OpenSpec CLI requires Node.js 20.19.0 or newer (the web application uses the
newer version in `.nvmrc`). Run the pinned CLI without a global install:

```sh
npx --yes @fission-ai/openspec@1.14.3 list
npx --yes @fission-ai/openspec@1.14.3 validate --all
```

Configuration lives in `openspec/config.yaml`; specifications live in
`openspec/specs/`, and active proposals live in `openspec/changes/`.
Reusable agent workflows are in `.agents/skills/`.

In clients that discover these skills, start with `/openspec-propose`; apply an
existing plan with `/openspec-apply-change`. You can also ask the agent to
follow the corresponding skill in `.agents/skills/`.
