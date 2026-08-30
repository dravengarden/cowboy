-- Browser-authorized, sender-constrained credentials for native and ACP clients.
-- Private keys and plaintext refresh tokens never enter SQLite.

CREATE TABLE user_devices (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 64),
    public_key TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    last_used_at_ms INTEGER,
    revoked_at_ms INTEGER,
    CHECK (length(id) = 32 AND id NOT GLOB '*[^0-9a-f]*')
);

CREATE INDEX user_devices_user_id_idx
    ON user_devices(user_id, created_at_ms DESC);

CREATE TABLE user_device_refresh_tokens (
    token_hash TEXT PRIMARY KEY,
    device_id TEXT NOT NULL REFERENCES user_devices(id) ON DELETE CASCADE,
    family_id TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    expires_at_ms INTEGER NOT NULL,
    used_at_ms INTEGER,
    revoked_at_ms INTEGER,
    CHECK (length(family_id) = 32 AND family_id NOT GLOB '*[^0-9a-f]*')
);

CREATE INDEX user_device_refresh_tokens_device_idx
    ON user_device_refresh_tokens(device_id, created_at_ms DESC);

CREATE INDEX user_device_refresh_tokens_expiry_idx
    ON user_device_refresh_tokens(expires_at_ms)
    WHERE used_at_ms IS NULL AND revoked_at_ms IS NULL;
