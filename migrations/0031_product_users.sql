-- Product users, login sessions, and unused API-token storage.
-- sessions.owner_user_id is shipped nullable and unread until the stamp PR.

CREATE TABLE users (
    id            text PRIMARY KEY,
    username      text NOT NULL UNIQUE,
    password_algo text NOT NULL,
    password_hash text NOT NULL,
    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now(),
    disabled_at   timestamptz
);

CREATE TABLE user_sessions (
    token_hash    text PRIMARY KEY,
    user_id       text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at    timestamptz NOT NULL DEFAULT now(),
    expires_at    timestamptz NOT NULL,
    last_seen_at  timestamptz NOT NULL DEFAULT now(),
    user_agent    text
);

CREATE INDEX user_sessions_user_id_idx ON user_sessions(user_id);
CREATE INDEX user_sessions_expires_idx ON user_sessions(expires_at);

CREATE TABLE user_api_tokens (
    id            text PRIMARY KEY,
    user_id       text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name          text NOT NULL,
    token_prefix  text NOT NULL,
    token_hash    text NOT NULL UNIQUE,
    created_at    timestamptz NOT NULL DEFAULT now(),
    expires_at    timestamptz,
    last_used_at  timestamptz,
    revoked_at    timestamptz
);

ALTER TABLE sessions
    ADD COLUMN owner_user_id text REFERENCES users(id) ON DELETE SET NULL;

CREATE INDEX sessions_owner_user_id_idx ON sessions(owner_user_id);
