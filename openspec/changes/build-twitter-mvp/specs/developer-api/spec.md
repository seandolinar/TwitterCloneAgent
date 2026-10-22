# Spec Delta

## Purpose

Let users automate the same social operations available in the browser through a documented API, without sharing their password or granting unrestricted account access.

## ADDED Requirements

### Requirement: Expose a versioned social API
The system SHALL expose social reads and writes under `/v1`, including current-user lookup, profile read/update, profile posts, home feed, post create/read/delete, follow/unfollow, and like/unlike. All third-party routes MUST require an API key, including reads of otherwise public content. Both browser and API requests MUST apply the same social validation and ownership rules.

#### Scenario: Publish through an integration
- **WHEN** a verified user's authorized integration creates a valid post
- **THEN** the post appears in the browser with that user as author and the same content limits apply

#### Scenario: Attempt credential management through the API
- **WHEN** an integration attempts to change a password, recover an account, or create another API key
- **THEN** no third-party endpoint permits the operation, regardless of the key's social permissions

### Requirement: Issue scoped user-owned Unkey keys
Only a verified user's browser session SHALL create API keys, after password reauthentication within the preceding five minutes. Keys MUST be created through Unkey, bound to the immutable local user ID, and expire within 365 days; the default expiry SHALL be 90 days. Supported permissions SHALL be `profiles.read`, `profiles.write`, `posts.read`, `posts.write`, `feed.read`, `follows.write`, and `likes.write`; the default selection MUST be read-only.

#### Scenario: Create a read-only key
- **WHEN** a verified user reauthenticates and creates a key with the default permissions
- **THEN** the key is associated only with that user and permits profile/post reads and the user's home feed
- **AND** it does not permit any write operation

#### Scenario: Attempt to choose another owner or unknown permission
- **WHEN** a key-creation request supplies another owner ID or unsupported permission
- **THEN** the caller cannot mint a key for the other account or acquire the unsupported permission

### Requirement: Reveal API secrets only once
The system SHALL display the full key only in its successful creation response, with caching disabled. Subsequent key lists MUST contain only non-secret identifiers, names, masked hints, permissions, expiry, and status. Full keys and Unkey management credentials MUST NOT be stored by the application, emitted in logs, or sent to frontend build assets or persistent browser storage.

#### Scenario: Revisit key settings
- **WHEN** a user returns to the key list after creating a key
- **THEN** they can identify the key but cannot retrieve its full secret again
- **AND** another user's key metadata is not visible

### Requirement: Revoke keys without a cache grace period
The system SHALL allow a browser-authenticated verified owner to revoke their keys and MUST deny those keys at the application immediately after local revocation commits. Provider revocation SHALL be retried durably if Unkey is unavailable. An unknown, expired, revoked, or owner-mismatched local key record MUST NOT authorize a request.

#### Scenario: Revoke during a provider outage
- **WHEN** an owner revokes a key while Unkey is unavailable or still caches its previous state
- **THEN** the local key record denies subsequent requests immediately
- **AND** provider revocation remains pending for retry without displaying the key as active

### Requirement: Authenticate through the Unkey gateway
The public third-party entry point MUST use Unkey's gateway to validate Bearer API keys in the configured keyspace before forwarding requests. The backend SHALL accept gateway identity only over an isolated trusted ingress and MUST validate its format, keyspace, key ID, local key ownership, expiry, and verified owner. Browser cookies or caller-supplied identity headers MUST NOT substitute for key authentication.

#### Scenario: Reject invalid credentials
- **WHEN** a request has no key, an invalid/expired key, a key from another keyspace, or no valid trusted gateway identity
- **THEN** it is rejected without invoking a social mutation

#### Scenario: Forge a gateway identity
- **WHEN** a caller supplies an `X-Unkey-Principal` header directly or through the browser proxy
- **THEN** the supplied identity cannot impersonate a user or bypass the gateway

### Requirement: Enforce permissions and bounded usage
Every third-party operation MUST enforce its documented permission and resource ownership. The gateway SHALL enforce a configurable per-key request limit, and social writes SHALL have a shared per-user limit across browser and API access. Missing permission MUST return 403 and exhausted limits MUST return 429 with a retry indication.

#### Scenario: Use a read-only key to delete a post
- **WHEN** an integration attempts a delete with no `posts.write` permission
- **THEN** the request receives 403 and no post changes
- **AND** a key with write permission still cannot delete another user's post

#### Scenario: Exceed the configured request limit
- **WHEN** a caller exceeds its key or shared user write limit
- **THEN** the operation is rejected with 429 and a retry indication before applying a write

### Requirement: Fail closed and document the API contract
If gateway authentication cannot establish authorization, the system MUST NOT fall back to accepting the request. The system SHALL publish an OpenAPI contract and examples covering endpoints, Bearer authentication, permissions, pagination, validation, and application versus gateway error formats. Secrets MUST be absent from examples.

#### Scenario: Authentication dependency is unavailable
- **WHEN** the gateway cannot verify a key or the backend cannot validate its local authorization record
- **THEN** protected operations do not execute and the caller receives a service error rather than authorized access

#### Scenario: Integrate using the published contract
- **WHEN** a developer follows the documented create-post, feed-pagination, or key-revocation example
- **THEN** the request and response shapes match the implementation and list the required permission
