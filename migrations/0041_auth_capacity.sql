-- Stable browser-session identities and database-authoritative interactive
-- client capacity. Cookie rotation changes token_hash but must never change the
-- logical session identity used for revocation, leases, and logout fan-out.

ALTER TABLE user_sessions
    ADD COLUMN session_id text;

UPDATE user_sessions
SET session_id = 'legacy-' || substr(token_hash, 1, 32)
WHERE session_id IS NULL;

ALTER TABLE user_sessions
    ALTER COLUMN session_id SET NOT NULL,
    ADD CONSTRAINT user_sessions_session_id_key UNIQUE (session_id),
    ADD COLUMN client_kind text NOT NULL DEFAULT 'browser',
    ADD COLUMN principal_class text NOT NULL DEFAULT 'human',
    ADD COLUMN revoked_at timestamptz,
    ADD COLUMN revoke_reason text,
    ADD COLUMN auth_provider_id text,
    ADD COLUMN auth_issuer text,
    ADD COLUMN auth_subject text,
    ADD COLUMN auth_sid text,
    ADD COLUMN id_token_ciphertext text,
    ADD CONSTRAINT user_sessions_client_kind_check CHECK (
        client_kind IN ('browser', 'native_shell')
    ),
    ADD CONSTRAINT user_sessions_principal_class_check CHECK (
        principal_class IN ('human', 'automation', 'system')
    ),
    ADD CONSTRAINT user_sessions_revoke_reason_check CHECK (
        revoke_reason IS NULL OR char_length(revoke_reason) BETWEEN 1 AND 128
    );

CREATE INDEX user_sessions_active_user_idx
    ON user_sessions(user_id, last_seen_at, created_at)
    WHERE revoked_at IS NULL;
CREATE INDEX user_sessions_provider_subject_idx
    ON user_sessions(auth_provider_id, auth_issuer, auth_subject)
    WHERE revoked_at IS NULL AND auth_provider_id IS NOT NULL;
CREATE INDEX user_sessions_provider_sid_idx
    ON user_sessions(auth_provider_id, auth_sid)
    WHERE revoked_at IS NULL AND auth_sid IS NOT NULL;

CREATE TABLE active_client_leases (
    client_id       text PRIMARY KEY,
    user_id         text REFERENCES users(id) ON DELETE CASCADE,
    principal_class text NOT NULL,
    session_id      text,
    client_kind     text NOT NULL,
    fencing_token   bigint NOT NULL CHECK (fencing_token > 0),
    acquired_at     timestamptz NOT NULL,
    heartbeat_at    timestamptz NOT NULL,
    expires_at      timestamptz NOT NULL,
    revoked_at      timestamptz,
    revoke_reason   text,
    CHECK (char_length(client_id) BETWEEN 16 AND 128),
    CHECK (principal_class IN ('human', 'automation', 'system')),
    CHECK (
        (principal_class = 'human' AND user_id IS NOT NULL)
        OR principal_class IN ('automation', 'system')
    ),
    CHECK (client_kind IN ('browser', 'native_shell', 'cli', 'acp', 'automation'))
);

CREATE INDEX active_client_leases_live_idx
    ON active_client_leases(principal_class, user_id, expires_at)
    WHERE revoked_at IS NULL;

CREATE TABLE capacity_waiters (
    waiter_id        text PRIMARY KEY,
    client_id        text NOT NULL UNIQUE,
    user_id          text REFERENCES users(id) ON DELETE CASCADE,
    principal_class  text NOT NULL,
    session_id       text,
    client_kind      text NOT NULL,
    requested_at     timestamptz NOT NULL,
    expires_at       timestamptz NOT NULL,
    reserved_until   timestamptz,
    CHECK (char_length(waiter_id) BETWEEN 16 AND 128),
    CHECK (char_length(client_id) BETWEEN 16 AND 128),
    CHECK (principal_class IN ('human', 'automation', 'system')),
    CHECK (
        (principal_class = 'human' AND user_id IS NOT NULL)
        OR principal_class IN ('automation', 'system')
    ),
    CHECK (client_kind IN ('browser', 'native_shell', 'cli', 'acp', 'automation'))
);

CREATE INDEX capacity_waiters_fair_idx
    ON capacity_waiters(principal_class, requested_at, waiter_id);

CREATE TABLE oidc_logout_jtis (
    provider_id text NOT NULL,
    jti         text NOT NULL,
    expires_at  timestamptz NOT NULL,
    PRIMARY KEY (provider_id, jti),
    CHECK (char_length(provider_id) BETWEEN 1 AND 128),
    CHECK (char_length(jti) BETWEEN 1 AND 512)
);

CREATE INDEX oidc_logout_jtis_expires_idx ON oidc_logout_jtis(expires_at);

CREATE TABLE auth_audit_events (
    id                bigserial PRIMARY KEY,
    occurred_at       timestamptz NOT NULL DEFAULT now(),
    event_type        text NOT NULL,
    actor_user_id     text REFERENCES users(id) ON DELETE SET NULL,
    principal_class   text NOT NULL,
    target_session_id text,
    detail            jsonb NOT NULL DEFAULT '{}'::jsonb,
    CHECK (char_length(event_type) BETWEEN 1 AND 128),
    CHECK (principal_class IN ('human', 'automation', 'system'))
);

CREATE INDEX auth_audit_events_occurred_idx
    ON auth_audit_events(occurred_at DESC);
