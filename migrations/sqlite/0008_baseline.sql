-- Consolidated SQLite baseline.
--
-- Fresh databases execute the former v1-v7 history in one SQLx transaction.
-- The compatibility ledger rows keep rollback to the immediately preceding
-- controller generation safe; existing v7 databases are marked at v8 by
-- the preflight in SqliteStorage::migrate and do not execute this file.

-- Former migration: 0001_baseline.sql

-- SQLite baseline for the complete Cowboy durable-storage contract.
--
-- PostgreSQL keeps its immutable incremental history in ../*.sql. SQLite was
-- introduced after schema v27, so a fresh local database starts directly from
-- the equivalent current schema. Timestamps are Unix milliseconds and JSON is
-- stored as validated TEXT; those representation choices stay private to the
-- SQLite backend.

CREATE TABLE machines (
    id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    connection_mode TEXT NOT NULL CHECK (connection_mode IN ('local', 'outbound_wss')),
    platform TEXT NOT NULL,
    architecture TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'offline'
        CHECK (status IN ('online', 'reconnecting', 'offline', 'updating', 'degraded')),
    protocol_min INTEGER NOT NULL DEFAULT 1,
    protocol_max INTEGER NOT NULL DEFAULT 1,
    inventory TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(inventory)),
    last_seen_at_ms INTEGER,
    created_at_ms INTEGER NOT NULL DEFAULT (CAST(strftime('%s', 'now') AS INTEGER) * 1000),
    updated_at_ms INTEGER NOT NULL DEFAULT (CAST(strftime('%s', 'now') AS INTEGER) * 1000),
    public_key TEXT,
    enrolled_at_ms INTEGER,
    revoked_at_ms INTEGER,
    connection_epoch TEXT,
    reconnect_deadline_at_ms INTEGER
);

INSERT INTO machines (
    id, display_name, connection_mode, platform, architecture, status
) VALUES (
    'hawk', 'Hawk', 'local', 'linux', 'x86_64', 'offline'
);

CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    machine_id TEXT NOT NULL DEFAULT 'hawk'
        REFERENCES machines(id) ON DELETE RESTRICT,
    workspace_id TEXT,
    workspace_name TEXT,
    workspace_source_path TEXT,
    cwd TEXT NOT NULL,
    title TEXT NOT NULL,
    origin TEXT NOT NULL,
    status TEXT NOT NULL,
    next_seq INTEGER NOT NULL DEFAULT 0,
    created_at_ms INTEGER NOT NULL DEFAULT (CAST(strftime('%s', 'now') AS INTEGER) * 1000),
    updated_at_ms INTEGER NOT NULL DEFAULT (CAST(strftime('%s', 'now') AS INTEGER) * 1000),
    agent_session_id TEXT,
    queue TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(queue)),
    drafts TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(drafts)),
    position REAL,
    deleted_at_ms INTEGER,
    auto_resume INTEGER,
    awaiting_user INTEGER NOT NULL DEFAULT 0,
    done INTEGER NOT NULL DEFAULT 0,
    judge_runs TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(judge_runs)),
    system INTEGER NOT NULL DEFAULT 0,
    mobile_review_state TEXT NOT NULL DEFAULT '{"mode":"git","tabs":[],"progress":{}}'
        CHECK (json_valid(mobile_review_state)),
    config_options TEXT CHECK (config_options IS NULL OR json_valid(config_options)),
    config_preferences TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(config_preferences))
);

CREATE INDEX sessions_machine_id_idx ON sessions(machine_id);
CREATE INDEX sessions_deleted_at_idx ON sessions(deleted_at_ms)
    WHERE deleted_at_ms IS NOT NULL;

CREATE TABLE events (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    ts_ms INTEGER NOT NULL DEFAULT (CAST(strftime('%s', 'now') AS INTEGER) * 1000),
    payload TEXT NOT NULL CHECK (json_valid(payload)),
    PRIMARY KEY (session_id, seq)
) WITHOUT ROWID;

CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL CHECK (json_valid(value)),
    updated_at_ms INTEGER NOT NULL DEFAULT (CAST(strftime('%s', 'now') AS INTEGER) * 1000)
);

CREATE TABLE scheduled_wakeups (
    session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
    fire_at_ms INTEGER NOT NULL,
    prompt TEXT NOT NULL
);

CREATE TABLE scheduled_provider_actions (
    provider TEXT PRIMARY KEY CHECK (provider = 'codex'),
    action TEXT NOT NULL CHECK (action = 'rate_limit_reset'),
    fire_at_ms INTEGER NOT NULL,
    idempotency_key TEXT NOT NULL,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    next_attempt_at_ms INTEGER
);

CREATE TABLE provider_action_logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    provider TEXT NOT NULL CHECK (provider = 'codex'),
    action TEXT NOT NULL CHECK (action = 'rate_limit_reset'),
    trigger TEXT NOT NULL CHECK (trigger IN ('manual', 'scheduled')),
    status TEXT NOT NULL CHECK (
        status IN ('scheduled', 'started', 'retrying', 'succeeded', 'failed', 'unknown', 'cancelled')
    ),
    phase TEXT NOT NULL,
    message TEXT NOT NULL,
    credit_id TEXT,
    idempotency_suffix TEXT,
    created_at_ms INTEGER NOT NULL
);

CREATE INDEX provider_action_logs_created_idx
    ON provider_action_logs(created_at_ms DESC);

CREATE TABLE machine_enrollment_tokens (
    token_hash TEXT PRIMARY KEY,
    machine_id TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    expires_at_ms INTEGER NOT NULL,
    used_at_ms INTEGER,
    created_at_ms INTEGER NOT NULL DEFAULT (CAST(strftime('%s', 'now') AS INTEGER) * 1000)
);

CREATE INDEX machine_enrollment_expiry_idx
    ON machine_enrollment_tokens(expires_at_ms)
    WHERE used_at_ms IS NULL;

CREATE TABLE runtime_incidents (
    id TEXT PRIMARY KEY,
    occurred_at_ms INTEGER NOT NULL,
    received_at_ms INTEGER NOT NULL DEFAULT (CAST(strftime('%s', 'now') AS INTEGER) * 1000),
    updated_at_ms INTEGER NOT NULL DEFAULT (CAST(strftime('%s', 'now') AS INTEGER) * 1000),
    source TEXT NOT NULL,
    classification TEXT NOT NULL,
    severity TEXT NOT NULL,
    state TEXT NOT NULL,
    summary TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    session_id TEXT,
    client_id TEXT,
    machine_id TEXT,
    trace_id TEXT,
    build TEXT,
    evidence_start_ms INTEGER NOT NULL,
    evidence_end_ms INTEGER NOT NULL,
    detail TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(detail)),
    recovered_at_ms INTEGER,
    recovery_outcome TEXT
);

CREATE INDEX runtime_incidents_occurred_idx
    ON runtime_incidents(occurred_at_ms DESC);
CREATE INDEX runtime_incidents_session_idx
    ON runtime_incidents(session_id, occurred_at_ms DESC)
    WHERE session_id IS NOT NULL;
CREATE INDEX runtime_incidents_fingerprint_idx
    ON runtime_incidents(fingerprint, occurred_at_ms DESC);
CREATE INDEX runtime_incidents_active_idx
    ON runtime_incidents(occurred_at_ms DESC)
    WHERE state <> 'recovered';

CREATE TABLE provider_usage_events (
    machine_id TEXT NOT NULL REFERENCES machines(id) ON DELETE RESTRICT,
    producer_id TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    occurred_at_ms INTEGER NOT NULL,
    received_at_ms INTEGER NOT NULL DEFAULT (CAST(strftime('%s', 'now') AS INTEGER) * 1000),
    account_fingerprint TEXT NOT NULL,
    provider TEXT NOT NULL CHECK (provider = 'deepseek'),
    agent TEXT NOT NULL,
    model TEXT,
    model_family TEXT NOT NULL DEFAULT 'unknown',
    resolved_model TEXT,
    model_revision TEXT,
    request_role TEXT NOT NULL DEFAULT 'unknown',
    status INTEGER NOT NULL CHECK (status BETWEEN 100 AND 599),
    input_tokens INTEGER,
    output_tokens INTEGER,
    reasoning_tokens INTEGER,
    cache_hit_tokens INTEGER,
    cache_miss_tokens INTEGER,
    schema_version INTEGER NOT NULL DEFAULT 1,
    operation TEXT NOT NULL DEFAULT 'legacy',
    protocol TEXT NOT NULL DEFAULT 'legacy',
    client_protocol TEXT NOT NULL DEFAULT 'legacy',
    upstream_protocol TEXT NOT NULL DEFAULT 'legacy',
    translation_mode TEXT NOT NULL DEFAULT 'legacy',
    thinking_mode TEXT NOT NULL DEFAULT 'unknown',
    reasoning_effort TEXT NOT NULL DEFAULT 'unknown',
    session_fingerprint TEXT,
    session_attribution TEXT NOT NULL DEFAULT 'unattributed',
    traffic_source TEXT NOT NULL DEFAULT 'unattributed',
    static_prefix_fingerprint TEXT,
    request_prefix_fingerprint TEXT,
    gateway_build TEXT,
    gateway_boot_id TEXT,
    cache_observation TEXT NOT NULL DEFAULT 'legacy',
    usage_observed INTEGER,
    completed INTEGER,
    streaming INTEGER,
    duration_ms INTEGER,
    request_bytes INTEGER,
    input_item_count INTEGER,
    tool_count INTEGER,
    system_block_count INTEGER,
    has_previous_response_id INTEGER,
    compatibility_fixes INTEGER,
    request_purpose TEXT NOT NULL DEFAULT 'interactive',
    cache_keepalive_outcome TEXT NOT NULL DEFAULT 'not_applicable',
    cache_keepalive_algorithm TEXT,
    cache_keepalive_attempt INTEGER,
    cache_keepalive_interval_ms INTEGER,
    cache_keepalive_source_age_ms INTEGER,
    source_request_prefix_fingerprint TEXT,
    PRIMARY KEY (machine_id, producer_id, sequence),
    CHECK (
        length(agent) BETWEEN 1 AND 32
        AND substr(agent, 1, 1) GLOB '[a-z]'
        AND agent NOT GLOB '*[^a-z0-9_-]*'
    ),
    CHECK (model_family IN ('unknown', 'flash', 'pro')),
    CHECK (request_role IN ('unknown', 'executor', 'planner', 'subagent', 'reviewer')),
    CHECK (operation IN ('legacy', 'responses', 'compact', 'messages', 'chat_completions')),
    CHECK (protocol IN ('legacy', 'responses', 'chat_completions', 'anthropic_messages')),
    CHECK (client_protocol IN ('legacy', 'responses', 'chat_completions', 'anthropic_messages')),
    CHECK (upstream_protocol IN ('legacy', 'responses', 'chat_completions', 'anthropic_messages')),
    CHECK (translation_mode IN ('legacy', 'native', 'responses_to_chat', 'anthropic_compat')),
    CHECK (thinking_mode IN ('unknown', 'enabled', 'disabled')),
    CHECK (reasoning_effort IN ('unknown', 'default', 'low', 'high', 'max')),
    CHECK (session_attribution IN ('unattributed', 'response_lineage', 'prefix_root', 'explicit')),
    CHECK (traffic_source IN ('unattributed', 'cowboy')),
    CHECK (
        (input_tokens IS NULL OR input_tokens BETWEEN 0 AND 10000000)
        AND (output_tokens IS NULL OR output_tokens BETWEEN 0 AND 10000000)
        AND (reasoning_tokens IS NULL OR reasoning_tokens BETWEEN 0 AND 10000000)
        AND (cache_hit_tokens IS NULL OR cache_hit_tokens BETWEEN 0 AND 10000000)
        AND (cache_miss_tokens IS NULL OR cache_miss_tokens BETWEEN 0 AND 10000000)
    ),
    CHECK (duration_ms IS NULL OR duration_ms BETWEEN 0 AND 86400000),
    CHECK (request_bytes IS NULL OR request_bytes BETWEEN 0 AND 67108864),
    CHECK (
        (input_item_count IS NULL OR input_item_count BETWEEN 0 AND 1000000)
        AND (tool_count IS NULL OR tool_count BETWEEN 0 AND 1000000)
        AND (system_block_count IS NULL OR system_block_count BETWEEN 0 AND 1000000)
        AND (compatibility_fixes IS NULL OR compatibility_fixes BETWEEN 0 AND 1000000)
    ),
    CHECK (usage_observed IS NULL OR usage_observed IN (0, 1)),
    CHECK (completed IS NULL OR completed IN (0, 1)),
    CHECK (streaming IS NULL OR streaming IN (0, 1)),
    CHECK (has_previous_response_id IS NULL OR has_previous_response_id IN (0, 1)),
    CHECK (
        session_fingerprint IS NULL OR (
            length(session_fingerprint) = 32
            AND session_fingerprint NOT GLOB '*[^0-9a-f]*'
        )
    ),
    CHECK (
        static_prefix_fingerprint IS NULL OR (
            length(static_prefix_fingerprint) = 32
            AND static_prefix_fingerprint NOT GLOB '*[^0-9a-f]*'
        )
    ),
    CHECK (
        request_prefix_fingerprint IS NULL OR (
            length(request_prefix_fingerprint) = 32
            AND request_prefix_fingerprint NOT GLOB '*[^0-9a-f]*'
        )
    ),
    CHECK (
        source_request_prefix_fingerprint IS NULL OR (
            length(source_request_prefix_fingerprint) = 32
            AND source_request_prefix_fingerprint NOT GLOB '*[^0-9a-f]*'
        )
    ),
    CHECK (
        gateway_build IS NULL OR (
            length(gateway_build) = 16
            AND gateway_build NOT GLOB '*[^0-9a-f]*'
        )
    ),
    CHECK (
        gateway_boot_id IS NULL OR (
            length(gateway_boot_id) = 16
            AND gateway_boot_id NOT GLOB '*[^0-9a-f]*'
        )
    ),
    CHECK (request_purpose IN ('interactive', 'cache_keepalive')),
    CHECK (
        cache_keepalive_outcome IN (
            'not_applicable', 'hit', 'miss', 'partial', 'retryable_error',
            'terminal_error', 'preempted'
        )
    ),
    CHECK (
        cache_keepalive_algorithm IS NULL OR (
            length(cache_keepalive_algorithm) BETWEEN 1 AND 64
            AND substr(cache_keepalive_algorithm, 1, 1) GLOB '[a-z]'
            AND cache_keepalive_algorithm NOT GLOB '*[^a-z0-9_-]*'
        )
    ),
    CHECK (cache_keepalive_attempt IS NULL OR cache_keepalive_attempt BETWEEN 0 AND 1000),
    CHECK (
        cache_keepalive_interval_ms IS NULL
        OR cache_keepalive_interval_ms BETWEEN 0 AND 604800000
    ),
    CHECK (
        cache_keepalive_source_age_ms IS NULL
        OR cache_keepalive_source_age_ms BETWEEN 0 AND 604800000
    ),
    CHECK (
        (
            request_purpose = 'interactive'
            AND cache_keepalive_outcome = 'not_applicable'
            AND cache_keepalive_algorithm IS NULL
            AND COALESCE(cache_keepalive_attempt, 0) = 0
            AND COALESCE(cache_keepalive_interval_ms, 0) = 0
            AND COALESCE(cache_keepalive_source_age_ms, 0) = 0
            AND source_request_prefix_fingerprint IS NULL
        ) OR (
            request_purpose = 'cache_keepalive'
            AND session_attribution = 'explicit'
            AND traffic_source = 'cowboy'
            AND cache_keepalive_outcome <> 'not_applicable'
            AND cache_keepalive_algorithm IS NOT NULL
            AND cache_keepalive_attempt >= 1
            AND cache_keepalive_interval_ms > 0
            AND cache_keepalive_source_age_ms >= 0
            AND source_request_prefix_fingerprint IS NOT NULL
        )
    )
) WITHOUT ROWID;

CREATE INDEX provider_usage_provider_time_idx
    ON provider_usage_events(provider, account_fingerprint, received_at_ms DESC);
CREATE INDEX provider_usage_agent_time_idx
    ON provider_usage_events(provider, agent, received_at_ms DESC);
CREATE INDEX provider_usage_machine_time_idx
    ON provider_usage_events(machine_id, received_at_ms DESC);
CREATE INDEX provider_usage_provider_occurred_idx
    ON provider_usage_events(provider, occurred_at_ms DESC);
CREATE INDEX provider_usage_model_family_time_idx
    ON provider_usage_events(provider, model_family, occurred_at_ms DESC);
CREATE INDEX provider_usage_role_time_idx
    ON provider_usage_events(provider, agent, request_role, occurred_at_ms DESC);
CREATE INDEX provider_usage_session_time_idx
    ON provider_usage_events(
        provider, account_fingerprint, agent, session_fingerprint, occurred_at_ms DESC
    ) WHERE session_fingerprint IS NOT NULL;
CREATE INDEX provider_usage_purpose_time_idx
    ON provider_usage_events(provider, request_purpose, occurred_at_ms DESC);
CREATE INDEX provider_usage_keepalive_outcome_time_idx
    ON provider_usage_events(provider, cache_keepalive_outcome, occurred_at_ms DESC)
    WHERE request_purpose = 'cache_keepalive';

CREATE TABLE provider_usage_producers (
    machine_id TEXT NOT NULL REFERENCES machines(id) ON DELETE RESTRICT,
    producer_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    account_fingerprint TEXT NOT NULL,
    agent TEXT NOT NULL,
    last_sequence INTEGER NOT NULL,
    last_occurred_at_ms INTEGER NOT NULL,
    last_received_at_ms INTEGER NOT NULL DEFAULT (CAST(strftime('%s', 'now') AS INTEGER) * 1000),
    PRIMARY KEY (machine_id, producer_id)
) WITHOUT ROWID;

-- Former migration: 0002_xai_provider_actions.sql

CREATE TABLE scheduled_provider_actions_next (
    provider TEXT PRIMARY KEY CHECK (provider IN ('codex', 'xai')),
    action TEXT NOT NULL CHECK (action = 'rate_limit_reset'),
    fire_at_ms INTEGER NOT NULL,
    idempotency_key TEXT NOT NULL,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    next_attempt_at_ms INTEGER
);

INSERT INTO scheduled_provider_actions_next
SELECT provider, action, fire_at_ms, idempotency_key, attempt_count, next_attempt_at_ms
FROM scheduled_provider_actions;

DROP TABLE scheduled_provider_actions;
ALTER TABLE scheduled_provider_actions_next RENAME TO scheduled_provider_actions;

CREATE TABLE provider_action_logs_next (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    provider TEXT NOT NULL CHECK (provider IN ('codex', 'xai')),
    action TEXT NOT NULL CHECK (action = 'rate_limit_reset'),
    trigger TEXT NOT NULL CHECK (trigger IN ('manual', 'scheduled')),
    status TEXT NOT NULL CHECK (
        status IN ('scheduled', 'started', 'retrying', 'succeeded', 'failed', 'unknown', 'cancelled')
    ),
    phase TEXT NOT NULL,
    message TEXT NOT NULL,
    credit_id TEXT,
    idempotency_suffix TEXT,
    created_at_ms INTEGER NOT NULL
);

INSERT INTO provider_action_logs_next
SELECT id, provider, action, trigger, status, phase, message, credit_id,
       idempotency_suffix, created_at_ms
FROM provider_action_logs;

DROP TABLE provider_action_logs;
ALTER TABLE provider_action_logs_next RENAME TO provider_action_logs;

CREATE INDEX provider_action_logs_created_idx
    ON provider_action_logs(created_at_ms DESC);

-- Former migration: 0003_provider_platform.sql

ALTER TABLE machines ADD COLUMN encryption_public_key TEXT;

ALTER TABLE sessions ADD COLUMN provider_version TEXT NOT NULL DEFAULT '';
ALTER TABLE sessions ADD COLUMN provider_generation_digest TEXT NOT NULL DEFAULT '';
ALTER TABLE sessions ADD COLUMN provider_auth_generation INTEGER;
ALTER TABLE sessions ADD COLUMN provider_behavior TEXT;
ALTER TABLE sessions ADD COLUMN purge_after_at_ms INTEGER;

CREATE INDEX sessions_provider_generation_idx
  ON sessions(machine_id, provider, provider_generation_digest)
  WHERE deleted_at_ms IS NULL;

CREATE INDEX sessions_purge_after_idx
  ON sessions(purge_after_at_ms)
  WHERE deleted_at_ms IS NOT NULL AND purge_after_at_ms IS NOT NULL;

-- Former migration: 0004_crash_incident_severity.sql

-- Keep SQLite diagnostics aligned with the PostgreSQL crash policy.
UPDATE runtime_incidents
SET severity = 'critical',
    classification = CASE
      WHEN lower(summary) LIKE '%did not become ready%'
        OR lower(summary) LIKE '%exited before readiness%'
        OR lower(summary) LIKE '%generation launch failed%'
      THEN 'worker_startup_failure'
      ELSE classification
    END,
    updated_at_ms = CAST(strftime('%s', 'now') AS INTEGER) * 1000
WHERE severity <> 'critical'
  AND (
    classification IN (
      'runtime_failure',
      'process_exit',
      'resource_exhaustion',
      'worker_startup_failure',
      'client_render_failure',
      'client_window_error',
      'client_unhandled_rejection'
    )
    OR (
      id LIKE 'lifecycle:%'
      AND classification IN ('protocol_failure', 'transport_failure')
    )
    OR lower(summary) LIKE '%did not become ready%'
    OR lower(summary) LIKE '%exited before readiness%'
    OR lower(summary) LIKE '%generation launch failed%'
  );

-- Former migration: 0005_product_users.sql

-- Product users, login sessions, and unused API-token storage.
-- sessions.owner_user_id is shipped nullable and unread until the stamp PR.

CREATE TABLE users (
    id TEXT PRIMARY KEY,
    username TEXT NOT NULL UNIQUE
        CHECK (
            username = lower(trim(username))
            AND length(username) BETWEEN 1 AND 64
            AND username NOT GLOB '*[^a-z0-9._-]*'
        ),
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

-- Former migration: 0006_user_passkeys.sql

-- Discoverable WebAuthn credentials and the product viewing lock clock.

ALTER TABLE users ADD COLUMN passkey_reauth_enabled INTEGER NOT NULL DEFAULT 1;
ALTER TABLE users ADD COLUMN last_step_up_at_ms INTEGER;

UPDATE users
SET last_step_up_at_ms = COALESCE(
    last_step_up_at_ms,
    updated_at_ms,
    CAST(strftime('%s', 'now') AS INTEGER) * 1000
);

CREATE TABLE user_passkeys (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    credential_id TEXT NOT NULL UNIQUE,
    nickname TEXT NOT NULL
        CHECK (length(nickname) BETWEEN 1 AND 64),
    passkey_json TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL DEFAULT (CAST(strftime('%s', 'now') AS INTEGER) * 1000),
    last_used_at_ms INTEGER
);

CREATE INDEX user_passkeys_user_id_idx ON user_passkeys(user_id);

-- Former migration: 0007_admin_passkeys.sql

-- Admin-plane WebAuthn credentials. Distinct from product user_passkeys.

CREATE TABLE admin_passkeys (
    id TEXT PRIMARY KEY,
    account TEXT NOT NULL,
    credential_id TEXT NOT NULL UNIQUE,
    nickname TEXT NOT NULL
        CHECK (length(nickname) BETWEEN 1 AND 64),
    passkey_json TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL DEFAULT (CAST(strftime('%s', 'now') AS INTEGER) * 1000),
    last_used_at_ms INTEGER
);

CREATE INDEX admin_passkeys_account_idx ON admin_passkeys(account);


-- Preserve predecessor rollback compatibility for databases first created
-- from this consolidated baseline. SQLx validates only version and checksum.
INSERT INTO _sqlx_migrations
    (version, description, success, checksum, execution_time)
VALUES
    (1, 'legacy compatibility', 1, X'37d5d2723cf03d0a97d735b7077481b798c15c331f6d9619f8a2c5516067de6c2d5210d756395bb8a9b4b6c06585daa3', 0),
    (2, 'legacy compatibility', 1, X'1998bf4bd51b478454c67b88f49c9898ee912c6b0e10c5c0bfc380039c70e2e93c9bf804f2e21ce6b927564c8e5e81ab', 0),
    (3, 'legacy compatibility', 1, X'95a4bf0af4e3dd9191833437926b53aac86e1135679bd6861e0becea6ba0bdb908b8741953a24ed88ef1ff2599518c4e', 0),
    (4, 'legacy compatibility', 1, X'6c3c72e4001e5c1c42cd06957a951f7eca965caaaa48b3bcda350498a8bc07d9f90f75348091884791f4972d99a1166d', 0),
    (5, 'legacy compatibility', 1, X'e3cbea9ed87d7ac30f6526fccec2fba8213bba0916d504e7a9af4d141ca0b39519aa2f7980ad34a1c44aa8b33e3c8ea7', 0),
    (6, 'legacy compatibility', 1, X'ea85426677156960acea20dd6b52d6d5a166bc2aa3a7a66bda848fef823c1253831b4f105480df42003ef4b5ce0faf1c', 0),
    (7, 'legacy compatibility', 1, X'015e64f500138d31ae413fa5855efbef9c592ef27c9e5631ff0059193a1e3f3a2355befbb277088635235f9b7b0bfe75', 0)
ON CONFLICT (version) DO NOTHING;
