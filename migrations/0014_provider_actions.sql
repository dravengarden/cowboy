-- One provider-level scheduled action. Reset credits are account-scoped, so a
-- session-owned timer could race another session and consume more than intended.
CREATE TABLE IF NOT EXISTS scheduled_provider_actions (
    provider        TEXT PRIMARY KEY,
    action           TEXT NOT NULL,
    fire_at_ms       BIGINT NOT NULL,
    idempotency_key  TEXT NOT NULL,
    CHECK (provider = 'codex'),
    CHECK (action = 'rate_limit_reset')
);
