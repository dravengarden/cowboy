-- Product users, login sessions, and unused API-token storage.
-- sessions.owner_user_id is shipped nullable and unread until the stamp PR.

CREATE TABLE users (
    id TEXT PRIMARY KEY,
    username TEXT NOT NULL UNIQUE,
    password_algo TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL DEFAULT (CAST(strftime('%s', 'now') AS INTEGER) * 1000),
    updated_at_ms INTEGER NOT NULL DEFAULT (CAST(strftime('%s', 'now') AS INTEGER) * 1000),
    disabled_at_ms INTEGER
);

CREATE TABLE user_sessions (
    token_hash TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at_ms INTEGER NOT NULL DEFAULT (CAST(strftime('%s', 'now') AS INTEGER) * 1000),
    expires_at_ms INTEGER NOT NULL,
    last_seen_at_ms INTEGER NOT NULL DEFAULT (CAST(strftime('%s', 'now') AS INTEGER) * 1000),
    user_agent TEXT
);

CREATE INDEX user_sessions_user_id_idx ON user_sessions(user_id);
CREATE INDEX user_sessions_expires_idx ON user_sessions(expires_at_ms);

CREATE TABLE user_api_tokens (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    token_prefix TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    created_at_ms INTEGER NOT NULL DEFAULT (CAST(strftime('%s', 'now') AS INTEGER) * 1000),
    expires_at_ms INTEGER,
    last_used_at_ms INTEGER,
    revoked_at_ms INTEGER
);

ALTER TABLE sessions ADD COLUMN owner_user_id TEXT REFERENCES users(id) ON DELETE SET NULL;

CREATE INDEX sessions_owner_user_id_idx ON sessions(owner_user_id);
