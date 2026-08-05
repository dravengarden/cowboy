-- Cowboy-owned provider usage ledger. Provider account facts remain in their
-- adapters; these rows describe only requests observed by Cowboy gateways.
CREATE TABLE provider_usage_events (
    machine_id text NOT NULL REFERENCES machines(id) ON DELETE RESTRICT,
    producer_id text NOT NULL,
    sequence bigint NOT NULL,
    occurred_at timestamptz NOT NULL,
    received_at timestamptz NOT NULL DEFAULT now(),
    account_fingerprint text NOT NULL,
    provider text NOT NULL,
    agent text NOT NULL,
    model text,
    status integer NOT NULL,
    input_tokens bigint,
    output_tokens bigint,
    reasoning_tokens bigint,
    cache_hit_tokens bigint,
    cache_miss_tokens bigint,
    schema_version integer NOT NULL DEFAULT 1,
    PRIMARY KEY (machine_id, producer_id, sequence),
    CHECK (provider IN ('deepseek')),
    CHECK (agent IN ('codex', 'claude')),
    CHECK (status BETWEEN 100 AND 599),
    CHECK (input_tokens IS NULL OR input_tokens >= 0),
    CHECK (output_tokens IS NULL OR output_tokens >= 0),
    CHECK (reasoning_tokens IS NULL OR reasoning_tokens >= 0),
    CHECK (cache_hit_tokens IS NULL OR cache_hit_tokens >= 0),
    CHECK (cache_miss_tokens IS NULL OR cache_miss_tokens >= 0)
);

CREATE INDEX provider_usage_provider_time_idx
    ON provider_usage_events (provider, account_fingerprint, received_at DESC);
CREATE INDEX provider_usage_agent_time_idx
    ON provider_usage_events (provider, agent, received_at DESC);
CREATE INDEX provider_usage_machine_time_idx
    ON provider_usage_events (machine_id, received_at DESC);

CREATE TABLE provider_usage_producers (
    machine_id text NOT NULL REFERENCES machines(id) ON DELETE RESTRICT,
    producer_id text NOT NULL,
    provider text NOT NULL,
    account_fingerprint text NOT NULL,
    agent text NOT NULL,
    last_sequence bigint NOT NULL,
    last_occurred_at timestamptz NOT NULL,
    last_received_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (machine_id, producer_id)
);
