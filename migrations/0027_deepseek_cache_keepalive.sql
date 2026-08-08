-- Distinguish background DeepSeek cache keepalives from interactive agent
-- requests. Keepalive rows retain only content-free request lineage and timing;
-- request snapshots stay process-local in the gateway and never enter Postgres.
ALTER TABLE provider_usage_events
    ADD COLUMN request_purpose text NOT NULL DEFAULT 'interactive',
    ADD COLUMN cache_keepalive_outcome text NOT NULL DEFAULT 'not_applicable',
    ADD COLUMN cache_keepalive_algorithm text,
    ADD COLUMN cache_keepalive_attempt bigint,
    ADD COLUMN cache_keepalive_interval_ms bigint,
    ADD COLUMN cache_keepalive_source_age_ms bigint,
    ADD COLUMN source_request_prefix_fingerprint text,
    ADD CONSTRAINT provider_usage_request_purpose_check
        CHECK (request_purpose IN ('interactive', 'cache_keepalive')),
    ADD CONSTRAINT provider_usage_cache_keepalive_outcome_check
        CHECK (cache_keepalive_outcome IN (
            'not_applicable', 'hit', 'miss', 'partial', 'retryable_error',
            'terminal_error', 'preempted'
        )),
    ADD CONSTRAINT provider_usage_cache_keepalive_algorithm_check
        CHECK (
            cache_keepalive_algorithm IS NULL OR
            cache_keepalive_algorithm ~ '^[a-z][a-z0-9_-]{0,63}$'
        ),
    ADD CONSTRAINT provider_usage_cache_keepalive_attempt_check
        CHECK (cache_keepalive_attempt IS NULL OR cache_keepalive_attempt BETWEEN 0 AND 1000),
    ADD CONSTRAINT provider_usage_cache_keepalive_interval_check
        CHECK (
            cache_keepalive_interval_ms IS NULL OR
            cache_keepalive_interval_ms BETWEEN 0 AND 604800000
        ),
    ADD CONSTRAINT provider_usage_cache_keepalive_source_age_check
        CHECK (
            cache_keepalive_source_age_ms IS NULL OR
            cache_keepalive_source_age_ms BETWEEN 0 AND 604800000
        ),
    ADD CONSTRAINT provider_usage_source_request_prefix_check
        CHECK (
            source_request_prefix_fingerprint IS NULL OR
            source_request_prefix_fingerprint ~ '^[0-9a-f]{32}$'
        ),
    ADD CONSTRAINT provider_usage_cache_keepalive_shape_check
        CHECK (
            (
                request_purpose = 'interactive' AND
                cache_keepalive_outcome = 'not_applicable' AND
                cache_keepalive_algorithm IS NULL AND
                coalesce(cache_keepalive_attempt, 0) = 0 AND
                coalesce(cache_keepalive_interval_ms, 0) = 0 AND
                coalesce(cache_keepalive_source_age_ms, 0) = 0 AND
                source_request_prefix_fingerprint IS NULL
            ) OR (
                request_purpose = 'cache_keepalive' AND
                session_attribution = 'explicit' AND
                traffic_source = 'cowboy' AND
                cache_keepalive_outcome <> 'not_applicable' AND
                cache_keepalive_algorithm IS NOT NULL AND
                cache_keepalive_attempt >= 1 AND
                cache_keepalive_interval_ms > 0 AND
                cache_keepalive_source_age_ms >= 0 AND
                source_request_prefix_fingerprint IS NOT NULL
            )
        );

CREATE INDEX provider_usage_purpose_time_idx
    ON provider_usage_events (provider, request_purpose, occurred_at DESC);
CREATE INDEX provider_usage_keepalive_outcome_time_idx
    ON provider_usage_events (provider, cache_keepalive_outcome, occurred_at DESC)
    WHERE request_purpose = 'cache_keepalive';

UPDATE sessions
SET config_preferences = config_preferences || '{"deepseek_cache_protection":true}'::jsonb
WHERE provider IN ('claude-deepseek', 'codex-deepseek')
  AND NOT config_preferences ? 'deepseek_cache_protection';
