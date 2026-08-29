ALTER TABLE users
    ADD COLUMN passkey_refresh_interval_ms bigint NOT NULL DEFAULT 604800000
        CHECK (passkey_refresh_interval_ms IN (86400000, 604800000, 1209600000));

ALTER TABLE users
    ALTER COLUMN passkey_reauth_enabled SET DEFAULT false;

UPDATE users SET passkey_reauth_enabled = false;

UPDATE user_sessions
SET expires_at = LEAST(expires_at, now() + interval '1 day');

ALTER TABLE user_sessions
    ADD COLUMN passkey_verified_at timestamptz;
