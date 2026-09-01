-- SQLite mirror of 0041_auth_capacity.sql. SQLite cannot tighten a newly
-- backfilled column to NOT NULL without rebuilding the table, so the unique
-- index plus write-path validation preserves the stable-ID invariant.

ALTER TABLE user_sessions ADD COLUMN session_id TEXT;
UPDATE user_sessions
SET session_id = 'legacy-' || substr(token_hash, 1, 32)
WHERE session_id IS NULL;
CREATE UNIQUE INDEX user_sessions_session_id_key ON user_sessions(session_id);

ALTER TABLE user_sessions ADD COLUMN client_kind TEXT NOT NULL DEFAULT 'browser'
    CHECK (client_kind IN ('browser', 'native_shell'));
ALTER TABLE user_sessions ADD COLUMN principal_class TEXT NOT NULL DEFAULT 'human'
    CHECK (principal_class IN ('human', 'automation', 'system'));
ALTER TABLE user_sessions ADD COLUMN revoked_at_ms INTEGER;
ALTER TABLE user_sessions ADD COLUMN revoke_reason TEXT
    CHECK (revoke_reason IS NULL OR length(revoke_reason) BETWEEN 1 AND 128);
ALTER TABLE user_sessions ADD COLUMN auth_provider_id TEXT;
ALTER TABLE user_sessions ADD COLUMN auth_issuer TEXT;
ALTER TABLE user_sessions ADD COLUMN auth_subject TEXT;
ALTER TABLE user_sessions ADD COLUMN auth_sid TEXT;
ALTER TABLE user_sessions ADD COLUMN id_token_ciphertext TEXT;

CREATE INDEX user_sessions_active_user_idx
    ON user_sessions(user_id, last_seen_at_ms, created_at_ms)
    WHERE revoked_at_ms IS NULL;
CREATE INDEX user_sessions_provider_subject_idx
    ON user_sessions(auth_provider_id, auth_issuer, auth_subject)
    WHERE revoked_at_ms IS NULL AND auth_provider_id IS NOT NULL;
CREATE INDEX user_sessions_provider_sid_idx
    ON user_sessions(auth_provider_id, auth_sid)
    WHERE revoked_at_ms IS NULL AND auth_sid IS NOT NULL;

CREATE TABLE active_client_leases (
    client_id       TEXT PRIMARY KEY CHECK (length(client_id) BETWEEN 16 AND 128),
    user_id         TEXT REFERENCES users(id) ON DELETE CASCADE,
    principal_class TEXT NOT NULL CHECK (principal_class IN ('human', 'automation', 'system')),
    session_id      TEXT,
    client_kind     TEXT NOT NULL CHECK (
        client_kind IN ('browser', 'native_shell', 'cli', 'acp', 'automation')
    ),
    fencing_token   INTEGER NOT NULL CHECK (fencing_token > 0),
    acquired_at_ms  INTEGER NOT NULL,
    heartbeat_at_ms INTEGER NOT NULL,
    expires_at_ms   INTEGER NOT NULL,
    revoked_at_ms   INTEGER,
    revoke_reason   TEXT,
    CHECK (
        (principal_class = 'human' AND user_id IS NOT NULL)
        OR principal_class IN ('automation', 'system')
    )
);

CREATE INDEX active_client_leases_live_idx
    ON active_client_leases(principal_class, user_id, expires_at_ms)
    WHERE revoked_at_ms IS NULL;

CREATE TABLE capacity_waiters (
    waiter_id       TEXT PRIMARY KEY CHECK (length(waiter_id) BETWEEN 16 AND 128),
    client_id       TEXT NOT NULL UNIQUE CHECK (length(client_id) BETWEEN 16 AND 128),
    user_id         TEXT REFERENCES users(id) ON DELETE CASCADE,
    principal_class TEXT NOT NULL CHECK (principal_class IN ('human', 'automation', 'system')),
    session_id      TEXT,
    client_kind     TEXT NOT NULL CHECK (
        client_kind IN ('browser', 'native_shell', 'cli', 'acp', 'automation')
    ),
    requested_at_ms INTEGER NOT NULL,
    expires_at_ms   INTEGER NOT NULL,
    reserved_until_ms INTEGER,
    CHECK (
        (principal_class = 'human' AND user_id IS NOT NULL)
        OR principal_class IN ('automation', 'system')
    )
);

CREATE INDEX capacity_waiters_fair_idx
    ON capacity_waiters(principal_class, requested_at_ms, waiter_id);

CREATE TABLE oidc_logout_jtis (
    provider_id TEXT NOT NULL CHECK (length(provider_id) BETWEEN 1 AND 128),
    jti         TEXT NOT NULL CHECK (length(jti) BETWEEN 1 AND 512),
    expires_at_ms INTEGER NOT NULL,
    PRIMARY KEY (provider_id, jti)
);

CREATE INDEX oidc_logout_jtis_expires_idx ON oidc_logout_jtis(expires_at_ms);

CREATE TABLE auth_audit_events (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    occurred_at_ms    INTEGER NOT NULL,
    event_type        TEXT NOT NULL CHECK (length(event_type) BETWEEN 1 AND 128),
    actor_user_id     TEXT REFERENCES users(id) ON DELETE SET NULL,
    principal_class   TEXT NOT NULL CHECK (principal_class IN ('human', 'automation', 'system')),
    target_session_id TEXT,
    detail_json       TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX auth_audit_events_occurred_idx
    ON auth_audit_events(occurred_at_ms DESC);
