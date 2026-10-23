# Browser entry point

See [the account walkthrough](ACCOUNT_GUIDE.md) for usable routes, local
registration/verification/recovery, and troubleshooting. Social screens and
integration-key settings remain pending.

TanStack Start/Nitro renders the browser product and forwards only an explicit
method/path allowlist to the Rust cookie listener:

- `/api/auth/*` -> `/browser/auth/*` for implemented account operations.
- `/api/v1/*` -> `/browser/v1/*` for implemented social operations.
- `/v1` and `/v1/*` are blocked in this ordinary browser-only mode.
- `/api/auth/keys` permits only browser key list/create and owner revocation;
  these never accept integration-key identity.
- There is no arbitrary URL, gateway, or catch-all backend forwarding route.

`API_BASE_URL` is server-only runtime configuration: a fixed HTTP(S) origin with
no credentials, path, query, or fragment. Compose sets `http://api:8080`;
standalone development defaults to `http://127.0.0.1:8080`. It is not a client
input or a frontend build-time secret.

## Trust boundary

The pinned `node-server` runtime supplies the actual Node request/socket. The
global Nitro middleware inspects the **raw** request target before router/URL
normalization. Encoded paths, dot segments, backslashes, double slashes, and
noncanonical targets are rejected; percent-encoded query cursors are allowed.
Unsupported/missing runtime peer context fails closed, never falling back to
caller forwarding headers. Compute-specific ingress is not enabled here.

Proxy headers are allowlisted. Only cookies, original Origin, CSRF header,
content type, and Accept are relayed. `Authorization`, `X-Unkey-*`, caller
request IDs, hop-by-hop headers, and public/private forwarding/IP headers do
not cross this boundary. `X-Twitterclone-Client-IP` is replaced with the socket
peer, normalizing IPv4-mapped IPv6 consistently. The proxy never synthesizes an
allowed Origin. Rust remains the only session/CSRF/business authorization
authority; an allowed proxy path does not authorize a request.

Body buffering is bounded to 16 KiB, and body read/backend fetch share an
eight-second deadline with cancellation. Backend redirects are not followed.
Responses preserve separate `Set-Cookie` headers and the safe response protocol
(content type, request ID, Allow, Retry-After), with no-store, nosniff, and
no-referrer protections. Transport/configuration failures produce a redacted
503, not an underlying URL/error dump.

SSR code must instantiate `createServerApi` within the current loader/server
function, passing that request. Its read-only closure snapshots only that
request's cookies/Origin and validates the same path/method allowlist. Do not
create a global API client/query cache containing a user's identity.

`getRouter` creates its own QueryClient and browser API closure for every SSR
request (and one for the hydrated browser router). TanStack's Query integration
owns dehydration/hydration. Only safe session DTOs enter the cache; cookies and
CSRF values do not. Login/logout/reset cancel and clear old queries before
reloading route state. Queries and writes do not retry automatically.

The typed API client bootstraps a signed CSRF context on first mutation,
deduplicates concurrent bootstrap calls, and replaces its memory-only token
after account rotation. Only an explicit `csrf_rejected` response can trigger
one refresh/retry, since Rust rejects that request before any mutation. Network
timeouts, 5xx, and other failures never replay a potentially committed write.
No password/session/CSRF/API-key value is persisted in browser storage.

## Rendering and response security

Account forms have visible labels/hints, keyboard focus, announced errors,
password clearing, and a pending-submit state. JavaScript-dependent submissions
stay disabled before hydration. Navigation wraps at narrow widths; text uses
React escaping and the shared plain-text component, never raw HTML.

The production Start entry generates a cryptographic nonce per request. That
same nonce is supplied to the router's SSR/hydration scripts and the response
CSP. Production permits neither `unsafe-inline` nor `unsafe-eval`; development
permits Vite's script/style/HMR needs explicitly. Frame embedding, arbitrary
form destinations, camera/microphone/geolocation use, MIME sniffing, referrer
leakage, and shared personalized response caching are restricted. The policy
uses `strict-dynamic` for trusted script loading, not as a claim that already
trusted JavaScript cannot create scripts. HTTPS/HSTS hosting-edge configuration
and the remaining Compute work are still pending.

## Validation

With the repository's pinned Node version:

```sh
npm ci --prefix apps/web
npm --prefix apps/web test
npm --prefix apps/web run test:proxy
npm --prefix apps/web run typecheck
```

The unit tests use isolated HTTP fixtures, not an application bypass.
They cover fixed origins, allowlisted methods/paths/query parameters,
identity/forwarding sanitization, original Origin preservation, separate
cookies, body limits, redirect denial, redacted errors, and simultaneous
two-user SSR read closures, CSRF refresh, request-scoped query state, and
account-change cache clearing.

`test:proxy` performs a production build, then starts the **built Nitro server**
against an isolated backend fixture. It sends raw HTTP encoded/traversal paths
and spoofed identity/client-IP headers, checks that blocked requests never
contact the backend, and verifies real separate Set-Cookie forwarding. This
also checks simultaneous personalized HTML from the actual Start renderer.
Real Chromium cookie/CSRF/SSR isolation checks use the real Compose stack:

```sh
PLAYWRIGHT_BROWSERS_PATH="$PWD/apps/web/node_modules/.cache/ms-playwright" \
  npm --prefix apps/web exec -- playwright install chromium
PLAYWRIGHT_BROWSERS_PATH="$PWD/apps/web/node_modules/.cache/ms-playwright" \
  python3 tests/with_compose.py npm --prefix apps/web run test:e2e
```

The wrapper generates runtime-only credentials, isolates images/ports/project
names, and stops its own containers/networks afterward without removing named
volumes. It does not touch a developer's `.env` or database. The tests exercise
real SMTP verification, two browser contexts, HttpOnly cookies, separate SSR
identity, and hostile-origin/missing-CSRF mutation denial. Browser traces,
screenshots, and video are disabled to avoid persisting passwords/link tokens.
They also exercise account screens, real recovery mail, old-session rejection,
320px keyboard forms, announced validation, pending state/password clearing, and
production CSP with successful hydration and blocked untrusted markup scripts.
Full-product acceptance and live-provider tests remain separate.

The Docker build copies both `src/` and `server/` through an explicit build
context allowlist. Credentials, test fixtures, and local dependencies do not
enter the runtime image.
