-- Admin-plane WebAuthn credentials. Distinct from product user_passkeys.

CREATE TABLE admin_passkeys (
    id            text PRIMARY KEY,
    account       text NOT NULL,
    credential_id text NOT NULL UNIQUE,
    nickname      text NOT NULL
                  CHECK (char_length(nickname) BETWEEN 1 AND 64),
    passkey_json  text NOT NULL,
    created_at    timestamptz NOT NULL DEFAULT now(),
    last_used_at  timestamptz
);

CREATE INDEX admin_passkeys_account_idx ON admin_passkeys(account);
