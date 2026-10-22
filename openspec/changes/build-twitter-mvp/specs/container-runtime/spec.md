# Spec Delta

## Purpose

Make the product reproducible to run and validate in containers while preserving private service boundaries and separating local conveniences from production security.

## ADDED Requirements

### Requirement: Start the local browser product with Compose
After copying the documented non-secret environment template, `docker compose up --build` SHALL start the frontend, backend, PostgreSQL, local mail catcher, and required schema initialization without host Rust or Node installations. The browser product MUST work without Unkey credentials; unavailable key-management features MUST clearly report that integration setup is required rather than issuing fake keys.

#### Scenario: Start a fresh checkout
- **WHEN** a developer follows the documented environment setup and runs `docker compose up --build` with an empty database volume
- **THEN** the browser becomes available at the documented loopback URL, the schema is ready, and verification/reset emails can be inspected locally

### Requirement: Exercise the real gateway locally
The system SHALL provide `docker compose --profile unkey up --build` for Unkey Frontline with route and key-authentication configuration. This mode MUST require a real development keyspace, narrowly scoped server credentials, and network access to Unkey; it MUST NOT imply that Unkey is fully hosted inside Compose.

#### Scenario: Start a configured gateway
- **WHEN** a developer supplies the documented Unkey development credentials and starts the `unkey` profile
- **THEN** the gateway accepts a valid user-owned key for `/v1` and rejects a request with no key before reaching the application

#### Scenario: Missing or invalid integration configuration
- **WHEN** the gateway profile is started without required configuration or cannot authenticate against Unkey
- **THEN** the integration reports a clear setup or dependency failure and never falls back to authorizing requests

### Requirement: Preserve private service ingress
The backend and database MUST NOT publish host ports in the standard Compose setup. Only the browser entry point, local mail UI, and optional third-party gateway SHALL publish loopback ports. The browser entry point MUST NOT forward third-party routes, and caller-supplied gateway identity headers MUST be removed before browser-origin traffic reaches the backend.

#### Scenario: Attempt direct backend access
- **WHEN** a caller outside the Compose networks tries to reach the backend or database through a host port
- **THEN** no standard published port provides that access
- **AND** the browser entry point cannot be used to inject a trusted gateway principal

### Requirement: Build reproducibly without embedding secrets
Application runtime images SHALL run as non-root users, use multi-stage builds and locked dependencies, and omit development tools where unnecessary. Credentials MUST be supplied only at runtime; environment templates SHALL contain placeholders, and real environment files MUST be ignored by version control. Production mode MUST reject insecure cookie settings, missing required secrets, and development mail delivery.

#### Scenario: Inspect a runtime image and configuration
- **WHEN** a developer builds the application images using the documented process
- **THEN** runtime users are non-root and application, database, SMTP, and Unkey secrets are not baked into image layers or frontend assets

#### Scenario: Start an unsafe production configuration
- **WHEN** production mode has insecure session cookies, missing secrets, or local-only mail settings
- **THEN** startup fails with a non-secret actionable error rather than silently using development defaults

### Requirement: Persist state and report readiness
The system SHALL persist PostgreSQL data in a named volume and apply versioned migrations before accepting application traffic. It MUST provide liveness and readiness endpoints, wait for database readiness during startup, and avoid destructive resets on restart. Readiness MUST fail while the database or required schema is unavailable without disclosing connection secrets.

#### Scenario: Restart the stack
- **WHEN** a developer restarts or recreates application containers without removing the data volume
- **THEN** accounts, posts, relationships, and local key-revocation state remain intact and migrations do not duplicate data

#### Scenario: Database or migration failure
- **WHEN** the database is unavailable or schema initialization fails
- **THEN** the application is not marked ready and does not accept partially initialized writes

### Requirement: Document the production boundary
The project SHALL document configuration, container commands, migrations, backups, TLS requirements, SMTP setup, and Unkey gateway operation. It MUST explicitly identify Frontline local-development mode and the mail catcher as non-production conveniences and describe a supported production gateway trust boundary without claiming that local Compose alone is production-ready.

#### Scenario: Prepare a production deployment
- **WHEN** an operator follows the deployment notes
- **THEN** the notes identify the remaining hosting, HTTPS, database backup, SMTP, and Unkey configuration work
- **AND** no step requires committing secrets, publicly publishing private source/images, or enabling a development authentication bypass
