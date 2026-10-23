# Browser entry point

TanStack Start/Nitro renders the browser product and forwards only an explicit
method/path allowlist to the Rust cookie listener:

- `/api/auth/*` -> `/browser/auth/*` for implemented account operations.
- `/api/v1/*` -> `/browser/v1/*` for implemented social operations.
- `/v1` and `/v1/*` are blocked in this ordinary browser-only mode.
- There is no arbitrary URL, gateway, credential-management, or catch-all backend
  forwarding route. Key-management routes will be added only with that feature.

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
create a global API client/query cache containing a user's identity. Typed
query/session state and browser screens are separate pending tasks.

## Validation

With the repository's pinned Node version:

```sh
npm ci --prefix apps/web
npm --prefix apps/web test
npm --prefix apps/web run test:proxy
npm --prefix apps/web run typecheck
```

The four unit tests use isolated HTTP fixtures, not an application bypass.
They cover fixed origins, allowlisted methods/paths/query parameters,
identity/forwarding sanitization, original Origin preservation, separate
cookies, body limits, redirect denial, redacted errors, and simultaneous
two-user SSR read closures.

`test:proxy` performs a production build, then starts the **built Nitro server**
against an isolated backend fixture. It sends raw HTTP encoded/traversal paths
and spoofed identity/client-IP headers, checks that blocked requests never
contact the backend, and verifies real separate Set-Cookie forwarding. This
proves the frontend boundary, not a complete Rust/browser-cookie workflow.
Full Compose/Playwright acceptance and live-provider tests remain separate.

The Docker build copies both `src/` and `server/` through an explicit build
context allowlist. Credentials, test fixtures, and local dependencies do not
enter the runtime image.
