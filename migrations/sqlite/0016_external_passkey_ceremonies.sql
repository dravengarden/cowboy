-- SQLite mirror of 0042_external_passkey_ceremonies.sql.

CREATE TABLE external_passkey_ceremonies (
    transaction_hash TEXT PRIMARY KEY CHECK (length(transaction_hash) = 64),
    ceremony_json     TEXT NOT NULL CHECK (json_valid(ceremony_json)),
    expires_at_ms     INTEGER NOT NULL,
    created_at_ms     INTEGER NOT NULL
);

CREATE INDEX external_passkey_ceremonies_expires_idx
    ON external_passkey_ceremonies(expires_at_ms);
