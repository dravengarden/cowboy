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
