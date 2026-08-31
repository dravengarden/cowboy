-- Move periodic Passkey verification to the shorter closed schedule. Keep the
-- predecessor columns as rollback ledgers because SQLite cannot replace their
-- CHECK constraints in place.

ALTER TABLE users
    ADD COLUMN passkey_verification_interval_ms INTEGER NOT NULL DEFAULT 86400000
        CHECK (
            passkey_verification_interval_ms IN (
                3600000,
                7200000,
                10800000,
                14400000,
                21600000,
                43200000,
                86400000,
                172800000,
                259200000
            )
        );

-- Preserve supported choices. Retired eight-hour settings become six hours;
-- retired seven- and fourteen-day settings adopt the safer one-day default.
UPDATE users
SET passkey_verification_interval_ms = CASE passkey_reauth_interval_ms
    WHEN 14400000 THEN 14400000
    WHEN 28800000 THEN 21600000
    WHEN 43200000 THEN 43200000
    WHEN 86400000 THEN 86400000
    WHEN 259200000 THEN 259200000
    ELSE 86400000
END;
