-- Discoverable WebAuthn credentials and the product viewing lock clock.

ALTER TABLE users
    ADD COLUMN passkey_reauth_enabled boolean NOT NULL DEFAULT true;

ALTER TABLE users
    ADD COLUMN last_step_up_at timestamptz;

UPDATE users
SET last_step_up_at = COALESCE(last_step_up_at, updated_at, now());

CREATE TABLE user_passkeys (
    id            text PRIMARY KEY,
    user_id       text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    credential_id text NOT NULL UNIQUE,
    nickname      text NOT NULL
                  CHECK (char_length(nickname) BETWEEN 1 AND 64),
    passkey_json  text NOT NULL,
    created_at    timestamptz NOT NULL DEFAULT now(),
    last_used_at  timestamptz
);

CREATE INDEX user_passkeys_user_id_idx ON user_passkeys(user_id);
