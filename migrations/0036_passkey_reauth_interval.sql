-- Add the canonical Passkey reauthentication interval without widening the
-- legacy refresh column. Keeping the old constrained value makes a rollback to
-- the preceding Controller safe after users select a new shorter interval.
ALTER TABLE users
    ADD COLUMN passkey_reauth_interval_ms bigint NOT NULL DEFAULT 259200000
        CHECK (
            passkey_reauth_interval_ms IN (
                14400000,
                28800000,
                43200000,
                86400000,
                259200000,
                604800000,
                1209600000
            )
        );

-- Preserve an intentional legacy choice. Accounts which never registered a
-- Passkey inherit the new three-day default instead of the old seven-day one.
UPDATE users
SET passkey_reauth_interval_ms = passkey_refresh_interval_ms
WHERE passkey_reauth_enabled
   OR passkey_refresh_interval_ms <> 604800000
   OR EXISTS (SELECT 1 FROM user_passkeys WHERE user_passkeys.user_id = users.id);
