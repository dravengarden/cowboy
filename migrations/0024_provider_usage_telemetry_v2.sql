-- Content-free request-shape telemetry for cache and reliability analysis.
-- Existing rows remain explicitly legacy and are excluded from verified cache
-- rates because version one could not distinguish absent from inferred data.
ALTER TABLE provider_usage_events
    ADD COLUMN operation text NOT NULL DEFAULT 'legacy',
    ADD COLUMN protocol text NOT NULL DEFAULT 'legacy',
    ADD COLUMN cache_observation text NOT NULL DEFAULT 'legacy',
    ADD COLUMN usage_observed boolean,
    ADD COLUMN completed boolean,
    ADD COLUMN streaming boolean,
    ADD COLUMN duration_ms bigint,
    ADD COLUMN request_bytes bigint,
    ADD COLUMN input_item_count bigint,
    ADD COLUMN tool_count bigint,
    ADD COLUMN system_block_count bigint,
    ADD COLUMN has_previous_response_id boolean,
    ADD COLUMN compatibility_fixes bigint,
    ADD CONSTRAINT provider_usage_operation_check
        CHECK (operation IN ('legacy', 'responses', 'compact', 'messages')),
    ADD CONSTRAINT provider_usage_protocol_check
        CHECK (protocol IN ('legacy', 'responses', 'chat_completions', 'anthropic_messages')),
    ADD CONSTRAINT provider_usage_cache_observation_check
        CHECK (cache_observation IN ('legacy', 'absent', 'derived', 'explicit')),
    ADD CONSTRAINT provider_usage_duration_check
        CHECK (duration_ms IS NULL OR duration_ms >= 0),
    ADD CONSTRAINT provider_usage_request_bytes_check
        CHECK (request_bytes IS NULL OR request_bytes >= 0),
    ADD CONSTRAINT provider_usage_input_item_count_check
        CHECK (input_item_count IS NULL OR input_item_count >= 0),
    ADD CONSTRAINT provider_usage_tool_count_check
        CHECK (tool_count IS NULL OR tool_count >= 0),
    ADD CONSTRAINT provider_usage_system_block_count_check
        CHECK (system_block_count IS NULL OR system_block_count >= 0),
    ADD CONSTRAINT provider_usage_compatibility_fixes_check
        CHECK (compatibility_fixes IS NULL OR compatibility_fixes BETWEEN 0 AND 1000000),
    ADD CONSTRAINT provider_usage_token_upper_bound_check
        CHECK (
            (input_tokens IS NULL OR input_tokens BETWEEN 0 AND 10000000) AND
            (output_tokens IS NULL OR output_tokens BETWEEN 0 AND 10000000) AND
            (reasoning_tokens IS NULL OR reasoning_tokens BETWEEN 0 AND 10000000) AND
            (cache_hit_tokens IS NULL OR cache_hit_tokens BETWEEN 0 AND 10000000) AND
            (cache_miss_tokens IS NULL OR cache_miss_tokens BETWEEN 0 AND 10000000)
        ),
    ADD CONSTRAINT provider_usage_duration_upper_bound_check
        CHECK (duration_ms IS NULL OR duration_ms <= 86400000),
    ADD CONSTRAINT provider_usage_request_bytes_upper_bound_check
        CHECK (request_bytes IS NULL OR request_bytes <= 67108864),
    ADD CONSTRAINT provider_usage_shape_upper_bound_check
        CHECK (
            (input_item_count IS NULL OR input_item_count <= 1000000) AND
            (tool_count IS NULL OR tool_count <= 1000000) AND
            (system_block_count IS NULL OR system_block_count <= 1000000)
        );

CREATE INDEX provider_usage_provider_occurred_idx
    ON provider_usage_events (provider, occurred_at DESC);
