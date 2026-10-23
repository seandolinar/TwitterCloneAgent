# Unkey Compute operator runbook

**Delivery, not deployment authorization.** The repository supplies a combined
image, environment/policy templates, and local checks. No project, repository
connection, paid dependency, domain, registry publication, or live deployment is
created by these instructions. Obtain separate authorization before those actions.
Unkey **Compute** hosts the app and gateway; **API Management** issues personal
keys. The local Frontline Compose profile is not a production Compute deployment.

## Topology and build

One non-root container runs the Node supervisor, TanStack Start/Nitro, and Rust.
Only Start listens externally, on `HOST=0.0.0.0` and platform `PORT` (default
8080). Rust's cookie listener is `127.0.0.1:8081`; its integration listener is
`127.0.0.1:8082`. No app-to-app public service calls or private-network assumption
is needed. The PostgreSQL server and SMTP service are external to the image.
Frontend/backend versions, resources, release, rollback, and scaling are coupled.

From the repository root, the local-only build is:

```sh
docker build -f infra/compute/Dockerfile -t twitterclone-combined:check .
```

The root `.dockerignore` is an allowlist; local env files, node_modules, target
directories, tests, Git history, certificates/private keys, and databases do not
enter the build context. Locked multi-stage builds use Rust 1.99.1 and Node
24.18.1. The runtime user is `node` (UID 1000). No build arguments or build-time
secrets are needed. Use runtime secret injection only.

After explicit hosting/source-upload authorization, prefer a **private GitHub
repository/Dockerfile build** with root directory `.` and Dockerfile
`infra/compute/Dockerfile`. A connected source provider must be approved to
receive this private source. Keep auto-deploy disabled; connecting a repository
or pushing a branch must not accidentally release it.

Alternatively, obtain separate registry/publication authorization before pushing
an image. Confirm the provider can authenticate to the selected private registry
or that public publication is explicitly acceptable; do not make a private
image/source public merely to satisfy image pull restrictions. Never use the
repository's `local` Git remote as a publication destination.

## Runtime and gateway templates

Versioned request-body templates:

| File | Documented control-plane operation |
| --- | --- |
| `infra/compute/runtime.production.json` | `POST /v2/environments.updateSettings`, production |
| `infra/compute/runtime.preview.json` | Same operation, preview |
| `infra/compute/gateway.production.json` | `POST /v2/gateway.setPolicies`, production |
| `infra/compute/gateway.preview.json` | Same operation, preview |
| `infra/compute/runtime.env.example` | Required private app runtime inputs, not an API request |

Replace `REPLACE_PROJECT` and the environment's placeholder keyspace only in a
reviewed operator copy. These files are **not** a provider auto-discovery manifest.
They have been compared with the current documented Compute fields, not submitted
to a workspace. No credential belongs in these JSON templates.
`gateway.setPolicies` replaces the **entire** environment policy list and changes
IDs: read/review the current list first, preserve required existing policies, and
obtain authorization for the exact replacement. Policies/settings apply to the
**next deployment**, not already running deployments. Recheck per environment
after deployment. Dashboard settings that write both environments are unsafe
for distinct keyspaces; configure production and preview separately.

The two disjoint key-auth policies match exactly `/v1` and prefix `/v1/`.
They accept Bearer keys from that environment's `ks_` namespace only. Browser
`/api/*`, assets, `/healthz`, `/readyz`, and `/docs/openapi.json` are outside
the key policies. Rust enforces endpoint scopes and ownership after verification.
Every issued key has one auto-applied 60/minute limit by default. Do not add
another verification, inline key limit, or gateway rate-limit policy counting
the same request. The app's separate shared per-user write budget defaults to
30/minute across browser and integration traffic.

The gateway strips caller `X-Unkey-*` headers and overwrites forwarding fields.
Compute mode accepts its single-IP `X-Forwarded-For` and `X-Forwarded-Proto:
https`, never a chain. Only known `/v1` methods/paths reach the loopback
integration listener, carrying the gateway principal but no cookie/raw Bearer.
The cookie branch strips principals/Bearer and preserves actual Origin/CSRF.
There is no authentication fallback between branches.

**Never expose the Compute-mode origin directly.** The principal is unsigned;
direct access would let a caller forge it. Loopback bindings, exclusive managed
gateway ingress, correct policy coverage, and header replacement are all required.
Do not turn on Compute mode for the ordinary publicly accessible browser service.

## Runtime secrets, PostgreSQL, and SMTP

The MVP supports `EMAIL_ENABLED=false` in Compute as well as locally. In that
mode no SMTP credential or sender-domain setup is required: account verification,
outgoing mail and email password recovery are disabled, and authenticated accounts
can use social/key features without mailbox verification. Existing verification
timestamps are preserved. Set the flag to true to restore the email requirements
below. Omission defaults to true for compatibility with existing deployments.
The app still requires an exact HTTPS browser origin; the hosting domain may be
used without configuring a separate email-sending domain.

Populate `infra/compute/runtime.env.example` only in private environment-specific
secret management. Do not commit a populated file, put secrets into API examples,
build assets, command arguments/shell history, request logs, or chat.

- `APP_ENV=production`, `NODE_ENV=production`, `WEB_RUNTIME_MODE=compute`.
- Use the supplied loopback bindings/upstream origins and external `HOST`.
  The platform supplies `PORT`; it must be 1–65535, distinct from both internal
  ports. Do not set `NITRO_PORT`, `NITRO_HOST`, or a Unix socket override.
- `APP_ORIGIN` is one exact external HTTPS browser origin, no credentials,
  path, query, fragment, or wildcard. Configure the actual environment domain
  before sending email links. `COOKIE_SECURE=true` is mandatory.
- `DATABASE_URL` is an external authenticated PostgreSQL URL with a nonempty
  database, username/password and **`sslmode=verify-full`**. `require` or
  `verify-ca` is insufficient. Only canonical `sslmode`, `sslrootcert`,
  `sslcert`, `sslkey`, and `application_name` query parameters are accepted;
  duplicates/aliases cannot silently lower TLS verification. URL-encode special
  credentials. Supply a trusted root with `sslrootcert` when the provider's CA
  is not in system trust. Certificate paths refer to private runtime mounts,
  never baked-in private keys. Review external database firewall/egress access.
- `CSRF_SECRET` and `RATE_LIMIT_SECRET`: two independently generated values of
  at least 32 bytes, distinct, stable across restarts, identical across replicas
  **within** this environment only. Never use known example values. Coordinate
  rotation across replicas; rate-secret rotation changes bucket identifiers.
- When `EMAIL_ENABLED=true`: `EMAIL_FROM`, external `SMTP_HOST`, SMTP port (usually 587), `SMTP_TLS=true`,
  and both nonblank SMTP credentials are required. The maintained client uses
  STARTTLS with certificate verification. A mail catcher/localhost is rejected.
  Configure sender/domain authorization, inbox delivery, spam handling, and
  provider limits separately. The local topology checks do not prove production
  SMTP deliverability.
- Supply environment-specific `UNKEY_MANAGEMENT_ROOT_KEY`, `UNKEY_API_ID`
  (`api_...`), and `UNKEY_KEYSPACE_ID` (`ks_...`). The management credential gets
  create/delete only for that namespace, no broad administration or recoverable
  key access. Compute's managed gateway owns verification; do not place its
  verify root in the app. A separate narrowly scoped operator credential for
  Compute settings is not an application runtime credential.

### Environment-isolation checklist

Before creating a deployment, independently compare production and preview:

- Different database names/data and database credentials, with each role denied
  access to the other's database. A shared managed server is acceptable only
  with independently enforced database access boundaries.
- Different Unkey API/keyspace IDs and root credentials, policies referencing
  only their own keyspace, and disposable preview personal keys.
- Different security secrets and exact origins; no preview default points to a
  production resource. Replicas in one environment share only that environment.
- Preview SMTP recipients/sandbox cannot deliver incidental mail to production
  users; secrets are scoped to the chosen environment, not inherited globally.
- Explicitly review which branch/source/image and domain each environment uses.

Syntax checks cannot prove these real resources are distinct. Local tests assert
separation of isolated fixture databases, keys, sessions, and secrets only.
Record non-secret resource identifiers and test outcomes during authorized setup.

## Startup, readiness, resources, and shutdown

The supervisor first runs the Rust `migrate` command with full configuration
validation. SQLx migration locking serializes concurrent instance startup; no
per-process leader is assumed. A failed connection/migration never starts either
server. After migrations, Rust and Start run together. Unexpected exit of either,
including exit zero, terminates the sibling and exits the container nonzero.
SIGTERM/SIGINT initiate shutdown; after ten seconds remaining children receive
SIGKILL. No child failure silently leaves half the product running.

`GET /healthz` checks frontend process liveness. `GET /readyz` calls the loopback
Rust schema/database readiness check with a bounded timeout; reaching it also
proves Start is running. It returns an empty 200 only for a ready process/schema,
otherwise empty 503, without connection/configuration details.

The supplied Compute health check deliberately uses **`/readyz`** as required by
this design. Provider guidance normally recommends process-only health because
failed probes restart instances. Here a database outage removes traffic and can
restart all replicas after six failed checks. This is a conscious readiness-over-
availability tradeoff, not a claim that restarts fix the database. Investigate
database incidents rather than adding a permissive health fallback.

Templates propose **1 vCPU / 1024 MiB** as a starting point, not a benchmark or a
promise that the plan supports it. SSR, the database pool (10/instance), bounded
Argon2id hashing, workers, and peak concurrent requests share resources. Measure
CPU/RSS, password latency, connection pressure, startup/migration duration, and
gateway timeouts before increasing replicas. Start in an authorized region near
the database; the templates intentionally do not replace region settings.
No durable state belongs on Compute's ephemeral filesystem. Keep local writes
below its 128 MiB default budget; logs go to stdout/stderr without request bodies,
headers, credential values, emails, or token query strings. Do not enable gateway
logging of auth/reset queries, cookies, bodies, or key-creation responses.

## Backups and schema-compatible rollback

Choose/authorize a PostgreSQL provider with encrypted backups and point-in-time
recovery. Set retention/RPO/RTO, restrict backup credentials, and perform a real
restore into an isolated preview database before launch. Protect backups as
private account data; this implementation does not provision or certify backups.

Before release, record the prior immutable image/deployment and migration version,
review schema compatibility, and take/verify a backup. A failed migration should
leave the new release unready. Inspect sanitized migration/version/checksum
information; do not drop/recreate tables, edit migration history, or wipe volumes
to bypass the failure. Schema changes must support rolling versions when traffic
can still hit the previous deployment.

If the previous application is compatible with the current schema, an explicitly
authorized rollback can move traffic to that deployment/image. Otherwise stop
writes and follow the rehearsed isolated backup/PITR restore procedure, including
an approved loss window and session/key-revocation implications. Do not roll back
revocations by accident or apply a destructive down migration automatically.
Recheck readiness, browser cookies, key denial, jobs, and email after rollback.

## Local evidence and separately authorized live checklist

```sh
python3 tests/test_compute_config.py
node --test tests/compute-supervisor.test.mjs
npm --prefix apps/web run test:proxy
python3 tests/test_compute_runtime.py
```

Offline checks cover template fields/matching and process supervision. Built
Nitro tests cover both ingress branches with HTTP fixtures. The combined-image
smoke script uses isolated Docker networks, TLS PostgreSQL and disposable
fixture identities/configuration; it does not upload source or contact Unkey.
It preserves named fixture data volumes and never touches a developer's `.env`.
Successful local results are **not** live Compute, hosted SMTP, or real Unkey
acceptance. Record the actual commands/results in the change evidence.
The latest [local validation report](../validation/twitter-mvp.md) records
executed checks and the limits of that evidence.

After **separate deployment authorization**, use this live checklist:

- [ ] Confirm approved workspace/plan, private source or registry access, resource
  spending limit, exact source/image, domain, region, and environment.
- [ ] Verify every real environment-isolation item above; record non-secret IDs.
- [ ] Apply reviewed settings and policy changes with scoped operator access,
  deploy, and verify the running deployment has those policies/settings.
- [ ] Observe one-time migrations and both supervised processes; `/readyz` 200,
  no direct origin/Rust exposure, and graceful replacement on SIGTERM.
- [ ] Exercise real SMTP registration/verification/recovery, cookie attributes,
  same-origin mutations, cross-user ownership, and two-user cache separation.
- [ ] Through the managed gateway, test no/invalid/wrong-keyspace/expired keys,
  least-permission denial, writes, cursor paging, 429/Retry-After, and immediate
  local revocation while provider propagation/cleanup is pending.
- [ ] Inspect verification analytics for one provider verification per request,
  not a Rust second call; do not store real key values in evidence.
- [ ] Exercise dependency/failure readiness and sibling termination in **preview**,
  not production without additional incident/test approval.
- [ ] Inspect assets/images/logging for secret exposure and prove preview writes
  do not touch production.
- [ ] Rehearse approved backup restore and schema-compatible rollback.

## Provider references checked for these templates

- [Dockerfile builds](https://www.unkey.com/docs/compute/build/dockerfile)
- [Runtime settings](https://www.unkey.com/docs/compute/configure/runtime-settings)
- [Update settings API](https://www.unkey.com/docs/platform/api-reference/environments/update-environment-settings)
- [Health checks](https://www.unkey.com/docs/compute/configure/health-checks)
- [Gateway policy matching](https://www.unkey.com/docs/compute/gateway/policies)
- [API key policy and auto-applied limits](https://www.unkey.com/docs/compute/gateway/api-key-auth)
- [Ingress headers/trust](https://www.unkey.com/docs/compute/networking/request-lifecycle)
- [Private networking limitations](https://www.unkey.com/docs/compute/networking/private-networking)
