-- Discoverable WebAuthn credentials and the product viewing lock clock.

ALTER TABLE users ADD COLUMN passkey_reauth_enabled INTEGER NOT NULL DEFAULT 1;
ALTER TABLE users ADD COLUMN last_step_up_at_ms INTEGER;

UPDATE users
SET last_step_up_at_ms = COALESCE(
    last_step_up_at_ms,
    updated_at_ms,
    CAST(strftime('%s', 'now') AS INTEGER) * 1000
);

CREATE TABLE user_passkeys (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    credential_id TEXT NOT NULL UNIQUE,
    nickname TEXT NOT NULL
        CHECK (length(nickname) BETWEEN 1 AND 64),
    passkey_json TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL DEFAULT (CAST(strftime('%s', 'now') AS INTEGER) * 1000),
    last_used_at_ms INTEGER
);

CREATE INDEX user_passkeys_user_id_idx ON user_passkeys(user_id);
