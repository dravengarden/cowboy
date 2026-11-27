-- Indexed turn-level cache telemetry projection surviving transcript Clear and
-- short-lived session purge (retained independently by occurred_at_ms).
CREATE TABLE IF NOT EXISTS turn_cache_facts (
    session_id TEXT NOT NULL,
    turn_end_seq BIGINT NOT NULL,
    occurred_at_ms BIGINT NOT NULL,
    owner_user_id TEXT,
    provider TEXT NOT NULL,
    provider_account_id TEXT NOT NULL,
    provider_version TEXT NOT NULL DEFAULT '',
    provider_generation_digest TEXT NOT NULL DEFAULT '',
    model TEXT,
    stop_reason TEXT NOT NULL,
    prompt_origin TEXT NOT NULL DEFAULT 'unknown',
    prev_turn_end_ms BIGINT,
    idle_gap_ms BIGINT,
    renewed BOOLEAN NOT NULL DEFAULT FALSE,
    compacted BOOLEAN NOT NULL DEFAULT FALSE,
    usage_observed BOOLEAN NOT NULL DEFAULT FALSE,
    total_tokens BIGINT,
    inclusive_input_tokens BIGINT,
    cached_read_tokens BIGINT,
    cache_write_tokens BIGINT,
    other_uncached_input_tokens BIGINT,
    output_tokens BIGINT,
    reasoning_tokens BIGINT,
    accounting_source TEXT NOT NULL DEFAULT 'turn_end',
    PRIMARY KEY (session_id, turn_end_seq)
);

CREATE INDEX IF NOT EXISTS turn_cache_facts_account_time_idx
    ON turn_cache_facts (owner_user_id, provider, provider_account_id, occurred_at_ms DESC);

CREATE INDEX IF NOT EXISTS turn_cache_facts_provider_time_idx
    ON turn_cache_facts (provider, occurred_at_ms DESC);

CREATE INDEX IF NOT EXISTS turn_cache_facts_session_time_idx
    ON turn_cache_facts (session_id, occurred_at_ms DESC);

CREATE INDEX IF NOT EXISTS turn_cache_facts_occurred_at_idx
    ON turn_cache_facts (occurred_at_ms);

CREATE TABLE IF NOT EXISTS turn_cache_backfill_state (
    slot INTEGER PRIMARY KEY CHECK (slot = 1),
    cursor_session_id TEXT NOT NULL DEFAULT '',
    cursor_seq BIGINT NOT NULL DEFAULT 0,
    completed_at_ms BIGINT,
    oldest_backfilled_ms BIGINT,
    rows_backfilled BIGINT NOT NULL DEFAULT 0,
    updated_at_ms BIGINT NOT NULL DEFAULT 0
);
