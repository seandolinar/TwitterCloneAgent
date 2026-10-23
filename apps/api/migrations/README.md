# Application schema

SQLx embeds the numbered SQL migrations into the backend binary. Keep the
migrations in version control and add a new numbered file for every schema
change. Never edit a migration that has been applied to a shared database:
SQLx checks its checksum.

Compose automatically runs the backend's one-shot `migrate` command before API
startup. To explicitly rerun it with the configured development environment:

```sh
docker compose build api
docker compose run --rm migrate
```

Successful reruns preserve data. Failed migrations prevent API startup. Inspect
the schema/version mismatch before retrying; do not delete a database volume or
edit applied migrations to make an error disappear. Back up shared databases
before schema changes and only roll back application images when compatible
with the installed schema.

The initial schema owns accounts, hashed session/account tokens, posts,
relationships, local Unkey key metadata/revocation, shared throttle buckets,
and durable email/provider-cleanup intents.

- Email normalization is enforced; handle uniqueness is case-insensitive.
- PostgreSQL character lengths count Unicode code points, not UTF-8 bytes.
- Follow/like pairs are unique; self-follow is prohibited. Post deletion
  cascades to likes. Service-layer ownership/verification checks are still
  required: database constraints do not authenticate an actor.
- Account tokens have an explicit purpose and at most one unconsumed token
  per user/purpose. Issuing a replacement must invalidate the previous token.
- Token/session records store 32-byte digests, never full bearer values.
- API keys store provider IDs, masked hints, allowlisted permissions, expiry,
  and revocation timestamps, never full secrets.
- Delivery jobs contain typed intents, not arbitrary payloads or token-bearing
  email bodies. Claim IDs and expiring leases support crash recovery.

Run the database integration test against a disposable PostgreSQL container:

```sh
python3 tests/with_postgres.py cargo test \
  --manifest-path apps/api/Cargo.toml --test database -- --ignored
```

The wrapper creates its own database, anonymous volume, and loopback test port;
it never uses the developer database. The test additionally refuses a database
whose name does not begin with `twitterclone_test_`. A normal `cargo test`
reports database tests as ignored; the command above is needed to run them.

The integration test checks uniqueness, foreign keys, text bounds, token/key/job
constraints, cascade deletion, and repeat migration without losing existing data.
