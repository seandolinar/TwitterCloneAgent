# Local account walkthrough

Account screens are usable now. The social API exists, but feed/profile/social
screens and Unkey key settings are still pending. This is a controlled MVP, not
a finished product or an unrestricted public deployment.

## Start the standard browser stack

Follow the root README: copy `.env.example` to a private `.env` if you have not
already done so, and generate `POSTGRES_PASSWORD`, `CSRF_SECRET`, and
`RATE_LIMIT_SECRET` independently. Never overwrite an existing private `.env`
without preserving its settings. Leave Unkey settings empty for browser use.

```sh
docker compose up --build
```

Default addresses:

- App: http://localhost:3000
- Local email inbox: http://localhost:8025
- Rust and PostgreSQL have no published host ports.

If you change `WEB_PORT`, set `APP_ORIGIN` to that exact external browser
origin too. Do not add a trailing path or use a different hostname in your
browser: `localhost` and `127.0.0.1` are different cookie/origin contexts.

## Register, verify, and log in

1. Open `/register` in a clean browser or private window.
2. Supply an email, a fixed 3–30-character ASCII handle (letters, digits,
   underscores), and a 15–128-code-point password. Password spaces are preserved.
3. Submit **Create account**. The acknowledgement is intentionally generic:
   it does not reveal whether the private email is already registered.
4. Open the local mail inbox and find the verification message for your address.
   Local Mailpit accepts test addresses; production needs real external SMTP.
5. Open the email link, then press **Verify email**. Merely opening it does not
   consume the token. Successful verification removes the token from the URL.
6. Use `/login` with the same email and exact password. The header shows the
   signed-in handle. Home currently reports account state; feed screens are
   a later task, not a broken login redirect.
7. To log out, open `/logout` and press **Confirm logout**. Visiting the page
   alone does not revoke your session.

An unverified user may log in, read public content, resend verification, and log
out, but cannot mutate social data, read a personalized feed, or create keys.
Use the header's **Verify email** link to request a resend while logged in.

## Reset a forgotten password

1. Open `/forgot-password`, enter the email, and submit **Send reset link**.
   Registered and unknown addresses receive the same acknowledgement.
2. Find the reset message in the local inbox and open its link.
3. Supply and confirm a valid new password, then press **Reset password**.
   Opening the link alone does not change the password or invalidate sessions.
4. A successful reset consumes the token, removes it from the URL, replaces the
   password, revokes all account sessions, and locally revokes all account keys
   while queueing provider cleanup. Other accounts are unaffected.
5. Log in again explicitly. Old passwords and old browser sessions must fail.
   Already-running requests are not cancelled by reset. Application request
   tests verify immediate local key denial; live provider acceptance remains separate.

## Browser routes

| Route | Behavior |
| --- | --- |
| `/` | Account-aware landing page and current implementation status |
| `/register` | Generic registration acknowledgement; no automatic login |
| `/login` | Email/password login with session and CSRF rotation |
| `/logout` | Explicit logout confirmation |
| `/verify-email?token=...` | Explicit one-time verification confirmation |
| `/verify-email` | Link/resend help; resend requires a browser session |
| `/forgot-password` | Generic reset-request acknowledgement |
| `/reset-password?token=...` | Explicit replacement-password form |
| `/reset-password` | Guidance to request/open a fresh reset link |
| `/healthz` | Frontend liveness, not full-product readiness |

Account UI calls `/api/auth/*`; social API calls use `/api/v1/*`.
Ordinary browser ingress blocks `/v1`. API requests still require the server's
authorization and mutation CSRF checks; navigation controls are not authorization.

## Troubleshooting

- **No message arrived:** check the intended address and local inbox. Delivery
  retries durably with backoff and may be delayed. A queued acknowledgement is
  not proof of SMTP delivery. Production requires an external STARTTLS SMTP
  endpoint, sender, and both SMTP credentials; the local mail catcher is not a
  production transport.
- **Link invalid/expired/already used:** verification lasts at most 24 hours,
  reset at most 30 minutes, and each token is single-use. A retry/resend replaces
  older tokens for that purpose. Open the newest message, or use the screen's
  fresh-link action. The UI does not claim success for a failed link.
- **Can't resend verification while signed out:** log in first, then use
  `/verify-email`. The acknowledgement is generic even if already verified.
- **Password rejected or won't match:** the policy counts Unicode code points,
  not bytes or UTF-16 units. No trimming occurs. Forms clear password fields
  after submission; enter them again after a validation or network failure.
- **Mutation rejected with 403:** use the configured browser origin. Do not
  fabricate Origin/CSRF headers or mix tabs/hosts manually. The client refreshes
  an expired CSRF context once after an explicit rejection. Other failures and
  ambiguous network errors do not automatically replay writes.
- **429 / too many attempts:** wait for the server's Retry-After interval.
  Authentication limits are shared across instances; they are not permanent
  account lockouts and cannot be bypassed by reloading.
- **Session ended:** inactivity expires after 24 hours, absolute lifetime after
  seven days; logout/reset revokes it sooner. A successful password reset always
  requires login again. Cookie values are HttpOnly, not readable by page scripts.
- **Lost inbox access:** the MVP has no administrator, social-login, or MFA
  recovery fallback. SMTP failure never invalidates the current password by itself.
- **Page won't load:** check Docker/API/database health and try the recovery
  button. The UI deliberately avoids showing underlying server errors or secrets.

Keep verification/reset URLs private. Responses use no-store and no-referrer;
there are no token-bearing analytics. Do not copy passwords/tokens into logs,
browser storage, chat, or source files. Production-build HTML uses a per-request
nonce CSP, anti-framing protections, and safe text rendering. Full production
TLS/Compute delivery remains pending; these controls do not authorize deployment.

## Verified workflow and limits of the evidence

The Chromium suite starts fresh browser contexts against real Compose
web/Rust/PostgreSQL/Mailpit services and follows these account steps:

```sh
PLAYWRIGHT_BROWSERS_PATH="$PWD/apps/web/node_modules/.cache/ms-playwright" \
  python3 tests/with_compose.py npm --prefix apps/web run test:e2e
```

It covers registration, real verification/reset email retrieval, explicit link
submission, resend, login/logout, old-session/password rejection, invalid/reused
links, generic acknowledgements, SSR isolation, cookie/CSRF security, and form
keyboard/mobile behavior. Backend tests separately exercise real expiration and
concurrent token/reset consumption; browser tests do not wait 24 hours to expire
a link. Tests do not prove live Unkey/Compute behavior or complete social UI
acceptance. The wrapper tears down its own stack and preserves its named volume.
Stop your ordinary stack with Ctrl+C or `docker compose down`; do not use `-v`
unless you explicitly intend to destroy its database data.
