ALTER TABLE users
    ADD COLUMN passkey_refresh_interval_ms INTEGER NOT NULL DEFAULT 604800000
        CHECK (passkey_refresh_interval_ms IN (86400000, 604800000, 1209600000));

UPDATE users SET passkey_reauth_enabled = 0;

UPDATE user_sessions
SET expires_at_ms = MIN(
    expires_at_ms,
    (CAST(strftime('%s', 'now') AS INTEGER) * 1000) + 86400000
);

ALTER TABLE user_sessions
    ADD COLUMN passkey_verified_at_ms INTEGER;
