# Spec Delta

## Purpose

Let people establish and recover an account without exposing credentials, and protect browser access with revocable sessions and abuse defenses.

## ADDED Requirements

### Requirement: Support the email-free MVP
The system SHALL support `EMAIL_ENABLED=false` in development and production.
In this mode email/password login SHALL work without mailbox verification;
registration SHALL NOT enqueue email, the SMTP worker SHALL NOT run, and email
verification/resend/password-reset request/consumption SHALL return 503 with
`email_disabled`. Existing verification timestamps SHALL remain unchanged.
New and existing accounts SHALL retain password/session/CSRF/throttling checks.
Omitting the setting defaults to true, restoring email behavior described below.
All mailbox-verification requirements below are conditional on enabled email.

#### Scenario: Register and use an account without email setup
- **WHEN** email is disabled and a person registers and logs in with valid credentials
- **THEN** they can use social and key features immediately without a verification email
- **AND** their email is not recorded as verified and no email intent is created

#### Scenario: Disable and re-enable email for existing accounts
- **WHEN** email is disabled for an existing unverified account
- **THEN** its existing session can use protected features without changing verification timestamps
- **AND** re-enabling email restores the verification requirement and recovery behavior

### Requirement: Register an account
The system SHALL accept an email address, unique case-insensitive handle, and password; enforce case-insensitive email uniqueness; and create an unverified account. Passwords MUST contain 15–128 Unicode code points, MUST NOT be silently trimmed or truncated, and MUST be stored only as salted, adaptive password hashes.

#### Scenario: New registration
- **WHEN** a visitor submits a valid unused email, a handle of 3–30 ASCII letters, digits, or underscores, and an acceptable password
- **THEN** an unverified account is created and a verification email is queued
- **AND** neither the plaintext password nor its hash appears in an API response or log

#### Scenario: Invalid or duplicate registration
- **WHEN** a registration has an invalid handle or password, or a previously registered email
- **THEN** invalid fields are rejected without creating a partial account
- **AND** a duplicate email receives the same public acknowledgement as an accepted registration without changing the existing account

### Requirement: Verify email ownership
The system SHALL verify an account only with an unexpired, single-use verification token valid for at most 24 hours. An unverified account SHALL be limited to public reads, verification resend, and logout; it MUST NOT perform social mutations, access a personalized feed, or create API keys.

#### Scenario: Verify an account
- **WHEN** the account owner submits the valid verification token
- **THEN** the account is marked verified and the token cannot be used again

#### Scenario: Expired token or unverified mutation
- **WHEN** an expired token is submitted, or an unverified session attempts a protected action
- **THEN** the action is rejected without modifying protected data
- **AND** the owner can request a fresh verification email subject to throttling

### Requirement: Authenticate with revocable sessions
The system SHALL authenticate email/password logins with opaque server-side sessions, rotate the session token on successful login, and expire sessions after 24 hours of inactivity or seven days from issuance, whichever occurs first. Production session cookies MUST be HttpOnly, Secure, SameSite=Lax, host-only, and scoped to `/`.

#### Scenario: Successful login
- **WHEN** an account supplies the correct credentials
- **THEN** the browser receives a new session cookie and subsequent authorized requests resolve to that account
- **AND** any pre-login session identifier is not retained

#### Scenario: Logout or expiration
- **WHEN** the owner logs out or a session reaches either expiry
- **THEN** further use of that token is rejected
- **AND** logout clears the cookie as well as revoking the server-side session

### Requirement: Recover access securely
The system SHALL acknowledge password-reset requests without revealing whether an email is registered. Reset tokens MUST be single-use, stored only as digests, and expire after at most 30 minutes. A successful reset MUST enforce the registration password policy, invalidate other reset tokens and all sessions, and locally revoke all of the account's API keys.

#### Scenario: Reset a forgotten password
- **WHEN** the owner submits a valid reset token and acceptable replacement password
- **THEN** only the new password works, old sessions and API keys no longer authorize requests, and the owner must log in again

#### Scenario: Invalid reset or email-delivery failure
- **WHEN** a reset token is expired or already used, or email cannot immediately be delivered
- **THEN** an invalid token cannot change the password
- **AND** a delivery failure does not invalidate the current password or sessions, and email delivery can be retried

### Requirement: Protect cookie-authenticated mutations
The system MUST reject browser state-changing requests unless they pass exact configured-origin checks and CSRF-token validation. This SHALL include registration, login, logout, verification, reset, social changes, and API-key management. A browser-supplied user ID or proxy identity header MUST NOT establish account identity.

#### Scenario: Cross-site mutation attempt
- **WHEN** an attacker submits a mutation from another origin, with a missing or invalid CSRF token, or with a forged identity header
- **THEN** the request cannot change account or social data, including when the browser has a valid session cookie

### Requirement: Bound authentication abuse and secret exposure
The system SHALL throttle login attempts by both normalized email and client IP, and throttle registration, reset requests, and verification resends by IP and relevant account identifier. Limits MUST be shared across backend instances. Unknown-email and wrong-password login failures MUST have the same public response and comparable password-check work. Authentication secrets MUST NOT appear in logs or public profiles.

#### Scenario: Repeated credential guessing
- **WHEN** login attempts exceed the configured account/IP or IP limit
- **THEN** further attempts receive HTTP 429 with a retry indication before another password hash is computed
- **AND** the limit expires rather than permanently locking the account

#### Scenario: Private account information
- **WHEN** another caller reads a profile or a request fails
- **THEN** the response and diagnostic logs do not expose email addresses, password hashes, session values, verification/reset tokens, or API-key secrets
