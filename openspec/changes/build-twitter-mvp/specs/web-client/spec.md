# Spec Delta

## Purpose

Provide a usable browser experience for elementary social interactions, account recovery, and safe management of personal integration keys.

## ADDED Requirements

### Requirement: Complete account workflows in the browser
The web client SHALL provide registration, login, logout, verification, verification resend, forgotten-password, and reset-password screens. It MUST explain unverified-account restrictions and recoverable errors without exposing whether a private email address is registered.

#### Scenario: First-time account onboarding
- **WHEN** a visitor registers and follows the delivered verification link
- **THEN** they can complete verification, log in, and reach their home feed
- **AND** before verification the UI explains why protected actions are unavailable

#### Scenario: Recover an expired verification or reset flow
- **WHEN** a person opens an expired or used link
- **THEN** the client shows a safe failure state and a route to request a fresh email instead of appearing to succeed

### Requirement: Browse and interact with the social product
The client SHALL provide a home feed, public profile with paginated posts, post-detail view, post composer, profile editor, follow/unfollow controls, like/unlike controls, and owner-only delete controls. The composer MUST show a 280-code-point limit consistent with server validation.

#### Scenario: Complete a social interaction
- **WHEN** a verified user publishes a post, follows another profile, likes a post, or deletes their own post
- **THEN** the corresponding views and counts reflect the confirmed server result
- **AND** a failed request does not leave a falsely successful UI state

#### Scenario: Browse while signed out
- **WHEN** a visitor opens an existing profile or post link
- **THEN** public content is readable and protected interactions invite login
- **AND** absent resources display a not-found state

### Requirement: Manage personal integration keys safely
The client SHALL provide a verified user's key list, a creation form with explicit permission and expiry selection, a password reauthentication step, a one-time secret display, and a revocation action. The default permission selection MUST be read-only. The full secret MUST disappear when the creation result is dismissed or the user navigates away.

#### Scenario: Create and revoke a key
- **WHEN** a verified owner creates a key after reauthentication
- **THEN** the client presents the secret once with a copy action and a warning that it cannot be recovered
- **AND** revocation updates the list to a revoked state even if provider cleanup remains pending

### Requirement: Preserve the browser security boundary
The client MUST render user content as plain text, keep session tokens out of JavaScript-accessible storage, and keep passwords and API-key secrets out of persistent browser storage. Personalized responses MUST NOT be shared between users through server-rendering caches. Requests to the backend MUST preserve the original origin and CSRF context rather than manufacture authorization.

#### Scenario: Render hostile text
- **WHEN** a post or biography contains markup or a script payload
- **THEN** the content is rendered as text without executing it

#### Scenario: Switch browser sessions
- **WHEN** two users request a personalized page, or one user logs out and another logs in
- **THEN** neither receives the other's cached feed, session information, or key-management result

### Requirement: Provide accessible and recoverable interactions
The client SHALL be usable at mobile and desktop widths, with keyboard-operable controls, labeled inputs, visible focus, and announced validation errors. Feed and profile lists MUST expose loading, empty, failure/retry, and end-of-list states and an accessible way to request another page.

#### Scenario: Keyboard and narrow-screen use
- **WHEN** a user navigates account forms and the feed with a keyboard at a narrow viewport
- **THEN** all primary actions remain reachable and labeled without horizontal page overflow

#### Scenario: Recover a failed page load
- **WHEN** a feed page fails to load
- **THEN** the client retains already loaded posts, explains the failure, and offers retry without duplicating posts
