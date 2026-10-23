CREATE TABLE users (
    id uuid PRIMARY KEY,
    email_normalized text NOT NULL UNIQUE,
    handle text NOT NULL CHECK (handle ~ '^[A-Za-z0-9_]{3,30}$'),
    handle_normalized text GENERATED ALWAYS AS (lower(handle)) STORED UNIQUE,
    password_hash text NOT NULL,
    verified_at timestamptz,
    display_name text NOT NULL CHECK (char_length(display_name) BETWEEN 1 AND 50),
    biography text NOT NULL DEFAULT '' CHECK (char_length(biography) <= 160),
    created_at timestamptz NOT NULL DEFAULT now(),
    CHECK (email_normalized = lower(btrim(email_normalized))),
    CHECK (char_length(email_normalized) BETWEEN 3 AND 254)
);

CREATE TABLE sessions (
    token_digest bytea PRIMARY KEY CHECK (octet_length(token_digest) = 32),
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    last_used_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    revoked_at timestamptz,
    password_authenticated_at timestamptz NOT NULL DEFAULT now(),
    CHECK (expires_at > created_at),
    CHECK (last_used_at >= created_at)
);
CREATE INDEX sessions_user_id ON sessions(user_id);
CREATE INDEX sessions_expiry ON sessions(expires_at);

CREATE TABLE account_tokens (
    token_digest bytea PRIMARY KEY CHECK (octet_length(token_digest) = 32),
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    purpose text NOT NULL CHECK (purpose IN ('email_verification', 'password_reset')),
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    consumed_at timestamptz,
    CHECK (expires_at > created_at)
);
CREATE UNIQUE INDEX account_tokens_current_purpose
    ON account_tokens(user_id, purpose) WHERE consumed_at IS NULL;
CREATE INDEX account_tokens_expiry ON account_tokens(expires_at);

CREATE TABLE posts (
    id uuid PRIMARY KEY,
    author_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    body text NOT NULL CHECK (
        char_length(body) BETWEEN 1 AND 280 AND body ~ '[^[:space:]]'
    ),
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX posts_author_cursor ON posts(author_id, created_at DESC, id DESC);
CREATE INDEX posts_feed_cursor ON posts(created_at DESC, id DESC);

CREATE TABLE follows (
    follower_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    followee_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (follower_id, followee_id),
    CHECK (follower_id <> followee_id)
);
CREATE INDEX follows_followee ON follows(followee_id);

CREATE TABLE likes (
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    post_id uuid NOT NULL REFERENCES posts(id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, post_id)
);
CREATE INDEX likes_post ON likes(post_id);

CREATE TABLE api_keys (
    key_id text PRIMARY KEY CHECK (key_id ~ '^key_'),
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    keyspace_id text NOT NULL CHECK (keyspace_id ~ '^ks_'),
    label text NOT NULL CHECK (char_length(label) BETWEEN 1 AND 100),
    masked_hint text NOT NULL CHECK (char_length(masked_hint) <= 32),
    permissions text[] NOT NULL CHECK (
        cardinality(permissions) BETWEEN 1 AND 7
        AND array_position(permissions, NULL) IS NULL
        AND permissions <@ ARRAY[
            'profiles.read', 'profiles.write', 'posts.read', 'posts.write',
            'feed.read', 'follows.write', 'likes.write'
        ]::text[]
    ),
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    revoked_at timestamptz,
    provider_revoked_at timestamptz,
    CHECK (expires_at > created_at),
    CHECK (expires_at <= created_at + interval '365 days'),
    CHECK (provider_revoked_at IS NULL OR revoked_at IS NOT NULL)
);
CREATE INDEX api_keys_owner ON api_keys(user_id, created_at DESC);

CREATE TABLE rate_limit_buckets (
    action text NOT NULL,
    identifier_digest bytea NOT NULL CHECK (octet_length(identifier_digest) = 32),
    window_start timestamptz NOT NULL,
    hits bigint NOT NULL CHECK (hits > 0),
    expires_at timestamptz NOT NULL,
    PRIMARY KEY (action, identifier_digest, window_start),
    CHECK (expires_at > window_start)
);
CREATE INDEX rate_limit_buckets_expiry ON rate_limit_buckets(expires_at);

-- Intent fields are deliberate: never persist token-bearing email bodies or keys.
CREATE TABLE delivery_jobs (
    id uuid PRIMARY KEY,
    kind text NOT NULL CHECK (
        kind IN ('verification_email', 'password_reset_email', 'unkey_revoke')
    ),
    user_id uuid REFERENCES users(id) ON DELETE CASCADE,
    key_id text,
    attempts integer NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    available_at timestamptz NOT NULL DEFAULT now(),
    lease_until timestamptz,
    claim_id uuid,
    created_at timestamptz NOT NULL DEFAULT now(),
    CHECK (
        (kind IN ('verification_email', 'password_reset_email')
            AND user_id IS NOT NULL AND key_id IS NULL)
        OR (kind = 'unkey_revoke' AND key_id IS NOT NULL)
    ),
    CHECK ((claim_id IS NULL) = (lease_until IS NULL))
);
CREATE INDEX delivery_jobs_ready ON delivery_jobs(available_at, id);
CREATE INDEX delivery_jobs_owner ON delivery_jobs(user_id);
