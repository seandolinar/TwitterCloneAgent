# Spec Delta

## Purpose

Provide the elementary social behavior of a text-first Twitter clone, with consistent ownership rules and bounded chronological browsing.

## ADDED Requirements

### Requirement: Read and edit public profiles
The system SHALL expose public profiles containing a handle, display name, biography, join date, follower/following counts, and post count. Only the verified profile owner SHALL edit the display name or biography. Display names MUST contain 1–50 Unicode code points and biographies at most 160; handles SHALL be immutable for this MVP.

#### Scenario: Read and update a profile
- **WHEN** a visitor opens an existing profile
- **THEN** the public fields and the account's posts are available without exposing its email or credentials
- **AND** a verified owner can save valid display-name and biography changes

#### Scenario: Reject another account's edit
- **WHEN** an authenticated account attempts to modify another profile
- **THEN** the system rejects the edit without changing that profile

### Requirement: Publish bounded plain-text posts
The system SHALL let verified users publish posts containing 1–280 Unicode code points, with at least one non-whitespace character. Posts MUST be treated as text rather than executable HTML. Posts SHALL have an immutable author, identifier, creation time, and body; post editing is outside this MVP.

#### Scenario: Publish a post
- **WHEN** a verified account submits a valid body
- **THEN** the saved post is publicly readable and attributed to the authenticated account, regardless of any author ID supplied by the caller

#### Scenario: Reject invalid content
- **WHEN** a caller submits an empty, whitespace-only, or over-280-code-point body
- **THEN** no post is created and the caller receives a validation error
- **AND** multibyte text is counted consistently by the browser and API

### Requirement: Delete only one's own posts
The system SHALL allow only the verified author to delete a post. A deleted post MUST disappear from direct reads, profiles, and feeds; its likes MUST no longer affect counts.

#### Scenario: Author deletes a post
- **WHEN** the verified author deletes an existing post
- **THEN** subsequent reads no longer return it and its like relationships no longer contribute to visible counts

#### Scenario: Non-author deletion
- **WHEN** another account attempts to delete the post
- **THEN** the request is rejected and the post remains unchanged

### Requirement: Follow other accounts
The system SHALL let verified users follow and unfollow existing accounts other than themselves. Following MUST have at most one relationship per ordered account pair; repeated follow or unfollow requests MUST be idempotent.

#### Scenario: Follow and unfollow
- **WHEN** a verified user follows another account and repeats the request
- **THEN** exactly one relationship exists and follower/following counts reflect one relationship
- **AND** unfollowing, including a repeated unfollow, leaves no relationship

#### Scenario: Invalid follow target
- **WHEN** the user attempts to follow themselves or a nonexistent account
- **THEN** no relationship is created and the caller receives a validation or not-found error

### Requirement: Read chronological cursor-paginated timelines
The system SHALL provide public per-profile post lists and a verified user's home feed containing their own posts and those of accounts they currently follow. Lists MUST sort by descending creation time with a deterministic identifier tie-breaker, use opaque cursors, default to 20 items, and cap requests at 100 items.

#### Scenario: Browse a home feed
- **WHEN** a verified account requests its home feed
- **THEN** only its own posts and currently followed accounts' posts appear, newest first
- **AND** an account following nobody still sees its own posts, or an empty state if it has none

#### Scenario: Advance a cursor
- **WHEN** a caller requests the next page using a returned cursor while new posts are added
- **THEN** previously returned posts do not repeat because of the insertions
- **AND** an invalid cursor or out-of-range page size receives a validation error

### Requirement: Like and unlike posts
The system SHALL let verified users like and unlike existing posts, with at most one like per user/post pair. These operations MUST be idempotent, and post reads MUST include the total like count and, when authenticated, whether the caller likes the post.

#### Scenario: Repeated likes and unlikes
- **WHEN** an account repeats a like or unlike request
- **THEN** the final relationship and count reflect one like or no like, not the number of requests

#### Scenario: Like a missing or deleted post
- **WHEN** an account attempts to like a missing or deleted post
- **THEN** the request returns not found and creates no relationship
