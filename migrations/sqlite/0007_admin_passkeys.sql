-- Admin-plane WebAuthn credentials. Distinct from product user_passkeys.

CREATE TABLE admin_passkeys (
    id TEXT PRIMARY KEY,
    account TEXT NOT NULL,
    credential_id TEXT NOT NULL UNIQUE,
    nickname TEXT NOT NULL
        CHECK (length(nickname) BETWEEN 1 AND 64),
    passkey_json TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL DEFAULT (CAST(strftime('%s', 'now') AS INTEGER) * 1000),
    last_used_at_ms INTEGER
);

CREATE INDEX admin_passkeys_account_idx ON admin_passkeys(account);
