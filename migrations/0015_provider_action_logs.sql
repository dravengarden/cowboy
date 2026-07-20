ALTER TABLE scheduled_provider_actions
    ADD COLUMN IF NOT EXISTS attempt_count INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS next_attempt_at_ms BIGINT;

UPDATE scheduled_provider_actions
SET next_attempt_at_ms = fire_at_ms
WHERE next_attempt_at_ms IS NULL;

CREATE TABLE IF NOT EXISTS provider_action_logs (
    id                  BIGSERIAL PRIMARY KEY,
    provider            TEXT NOT NULL,
    action              TEXT NOT NULL,
    trigger             TEXT NOT NULL,
    status              TEXT NOT NULL,
    phase               TEXT NOT NULL,
    message             TEXT NOT NULL,
    credit_id           TEXT,
    idempotency_suffix  TEXT,
    created_at_ms       BIGINT NOT NULL,
    CHECK (provider = 'codex'),
    CHECK (action = 'rate_limit_reset'),
    CHECK (trigger IN ('manual', 'scheduled')),
    CHECK (status IN ('scheduled', 'started', 'retrying', 'succeeded', 'failed', 'unknown', 'cancelled'))
);

CREATE INDEX IF NOT EXISTS provider_action_logs_created_idx
    ON provider_action_logs (created_at_ms DESC);
