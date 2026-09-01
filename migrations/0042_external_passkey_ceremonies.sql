-- Restart-safe, PKCE-bound Passkey handoff transactions. Only the hash of the
-- bearer transaction id is persisted; WebAuthn challenge state and terminal
-- results remain replayable for the short ceremony lifetime.

CREATE TABLE external_passkey_ceremonies (
    transaction_hash text PRIMARY KEY,
    ceremony_json     jsonb NOT NULL,
    expires_at        timestamptz NOT NULL,
    created_at        timestamptz NOT NULL,
    CHECK (char_length(transaction_hash) = 64)
);

CREATE INDEX external_passkey_ceremonies_expires_idx
    ON external_passkey_ceremonies(expires_at);
