-- Browser-authorized, sender-constrained credentials for native and ACP clients.
-- Private keys and plaintext refresh tokens never enter PostgreSQL.

CREATE TABLE user_devices (
    id            text PRIMARY KEY,
    user_id       text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name          text NOT NULL CHECK (char_length(name) BETWEEN 1 AND 64),
    public_key    text NOT NULL,
    created_at    timestamptz NOT NULL DEFAULT now(),
    last_used_at  timestamptz,
    revoked_at    timestamptz,
    CHECK (id ~ '^[0-9a-f]{32}$')
);

CREATE INDEX user_devices_user_id_idx
    ON user_devices(user_id, created_at DESC);

CREATE TABLE user_device_refresh_tokens (
    token_hash    text PRIMARY KEY,
    device_id     text NOT NULL REFERENCES user_devices(id) ON DELETE CASCADE,
    family_id     text NOT NULL,
    created_at    timestamptz NOT NULL DEFAULT now(),
    expires_at    timestamptz NOT NULL,
    used_at       timestamptz,
    revoked_at    timestamptz,
    CHECK (family_id ~ '^[0-9a-f]{32}$')
);

CREATE INDEX user_device_refresh_tokens_device_idx
    ON user_device_refresh_tokens(device_id, created_at DESC);

CREATE INDEX user_device_refresh_tokens_expiry_idx
    ON user_device_refresh_tokens(expires_at)
    WHERE used_at IS NULL AND revoked_at IS NULL;
