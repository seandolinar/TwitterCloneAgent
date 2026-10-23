# Spec Delta

## Purpose

Make the product reproducible to run and validate in containers while preserving private service boundaries and separating local conveniences from production security.

## ADDED Requirements

Email policy amendment: the MVP templates select `EMAIL_ENABLED=false`; when
disabled no SMTP credentials, sender domain or outbound mail service is required,
including in production/Compute. Email-specific startup, delivery and SMTP
requirements below apply when enabled. Local Mailpit may remain running for
optional future use. Other production requirements, including HTTPS origin,
verified database TLS, private ingress and runtime secrets, remain mandatory.

### Requirement: Run without an email provider
The runtime SHALL accept a production configuration with `EMAIL_ENABLED=false`
and absent SMTP settings while enforcing all other security configuration.

#### Scenario: Start the email-free production configuration
- **WHEN** a valid production configuration disables email and omits SMTP credentials
- **THEN** startup succeeds without constructing an email transport
- **AND** disabling email cannot allow insecure cookies, database TLS or missing application secrets

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
The backend and database MUST NOT publish host ports in the standard Compose setup. Only the browser entry point, local mail UI, and optional third-party gateway SHALL publish loopback ports. The local browser-only entry point MUST NOT forward third-party routes, and caller-supplied gateway identity headers MUST be removed before browser-origin traffic reaches the backend.

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
The project SHALL provide an Unkey Compute deployment runbook covering configuration, migrations, backups, TLS, SMTP, gateway policies, and rollback. It MUST identify Frontline local-development mode and the mail catcher as local-only conveniences. Actual provisioning, publication, and deployment MUST require separate authorization.

#### Scenario: Prepare a production deployment
- **WHEN** an operator follows the deployment notes
- **THEN** the notes identify the remaining hosting, HTTPS, database backup, SMTP, and Unkey configuration work
- **AND** no step requires committing secrets, publicly publishing private source/images, or enabling a development authentication bypass

### Requirement: Deliver a combined Unkey Compute application
The system SHALL provide a non-root production image running the frontend and Rust in one Unkey Compute app. Only one ingress SHALL listen on the platform's `PORT`; both Rust interfaces MUST bind loopback. The production image MUST exclude the development database and mail catcher.

#### Scenario: Run the production image
- **WHEN** the combined image starts with a valid production configuration and platform port
- **THEN** browser and integration requests use the same gateway-facing ingress
- **AND** callers cannot connect directly to either Rust interface to bypass ingress controls

### Requirement: Supervise production processes and schema readiness
The production entrypoint MUST apply versioned migrations before accepting traffic, supervise both application processes, and propagate shutdown signals. Readiness MUST fail for an unavailable database, incompatible schema, or failed child process. Production database connections MUST use authenticated, certificate-verified TLS.

#### Scenario: Startup or runtime dependency failure
- **WHEN** a migration fails, a required process exits, or the database becomes unavailable
- **THEN** the deployment does not report ready or accept partially initialized writes
- **AND** a failed child causes the combined deployment to terminate rather than silently serve half an application

### Requirement: Isolate production and preview environments
Production and preview MUST use separate PostgreSQL databases/data, Unkey keyspaces and scoped credentials, application secrets, and exact browser origins. Secrets MUST be injected at runtime. Replicas within one environment MUST share that environment's stable security configuration and durable state.

#### Scenario: Exercise a preview deployment
- **WHEN** a preview environment is configured and exercised
- **THEN** its reads, writes, email delivery, and API keys do not use production data or credentials
- **AND** the runbook identifies the isolation checks without embedding secret values
