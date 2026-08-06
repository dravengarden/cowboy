-- Provider-neutral request lineage and prefix telemetry. Fingerprints are
-- runtime-namespaced, credential-keyed HMACs produced by each isolated gateway; no prompt,
-- response, session id, tool payload, or credential is stored.
ALTER TABLE provider_usage_events
    DROP CONSTRAINT provider_usage_events_agent_check,
    DROP CONSTRAINT provider_usage_operation_check,
    ADD COLUMN model_family text NOT NULL DEFAULT 'unknown',
    ADD COLUMN resolved_model text,
    ADD COLUMN model_revision text,
    ADD COLUMN request_role text NOT NULL DEFAULT 'unknown',
    ADD COLUMN client_protocol text NOT NULL DEFAULT 'legacy',
    ADD COLUMN upstream_protocol text NOT NULL DEFAULT 'legacy',
    ADD COLUMN translation_mode text NOT NULL DEFAULT 'legacy',
    ADD COLUMN thinking_mode text NOT NULL DEFAULT 'unknown',
    ADD COLUMN reasoning_effort text NOT NULL DEFAULT 'unknown',
    ADD COLUMN session_fingerprint text,
    ADD COLUMN session_attribution text NOT NULL DEFAULT 'unattributed',
    ADD COLUMN traffic_source text NOT NULL DEFAULT 'unattributed',
    ADD COLUMN static_prefix_fingerprint text,
    ADD COLUMN request_prefix_fingerprint text,
    ADD COLUMN gateway_build text,
    ADD COLUMN gateway_boot_id text,
    ADD CONSTRAINT provider_usage_agent_slug_check
        CHECK (agent ~ '^[a-z][a-z0-9_-]{0,31}$'),
    ADD CONSTRAINT provider_usage_operation_v3_check
        CHECK (operation IN ('legacy', 'responses', 'compact', 'messages', 'chat_completions')),
    ADD CONSTRAINT provider_usage_model_family_check
        CHECK (model_family IN ('unknown', 'flash', 'pro')),
    ADD CONSTRAINT provider_usage_request_role_check
        CHECK (request_role IN ('unknown', 'executor', 'planner', 'subagent', 'reviewer')),
    ADD CONSTRAINT provider_usage_client_protocol_check
        CHECK (client_protocol IN ('legacy', 'responses', 'chat_completions', 'anthropic_messages')),
    ADD CONSTRAINT provider_usage_upstream_protocol_check
        CHECK (upstream_protocol IN ('legacy', 'responses', 'chat_completions', 'anthropic_messages')),
    ADD CONSTRAINT provider_usage_translation_mode_check
        CHECK (translation_mode IN ('legacy', 'native', 'responses_to_chat', 'anthropic_compat')),
    ADD CONSTRAINT provider_usage_thinking_mode_check
        CHECK (thinking_mode IN ('unknown', 'enabled', 'disabled')),
    ADD CONSTRAINT provider_usage_reasoning_effort_check
        CHECK (reasoning_effort IN ('unknown', 'default', 'low', 'high', 'max')),
    ADD CONSTRAINT provider_usage_session_attribution_check
        CHECK (session_attribution IN ('unattributed', 'response_lineage', 'prefix_root', 'explicit')),
    ADD CONSTRAINT provider_usage_traffic_source_check
        CHECK (traffic_source IN ('unattributed', 'cowboy')),
    ADD CONSTRAINT provider_usage_session_fingerprint_check
        CHECK (session_fingerprint IS NULL OR session_fingerprint ~ '^[0-9a-f]{32}$'),
    ADD CONSTRAINT provider_usage_static_prefix_fingerprint_check
        CHECK (static_prefix_fingerprint IS NULL OR static_prefix_fingerprint ~ '^[0-9a-f]{32}$'),
    ADD CONSTRAINT provider_usage_request_prefix_fingerprint_check
        CHECK (request_prefix_fingerprint IS NULL OR request_prefix_fingerprint ~ '^[0-9a-f]{32}$'),
    ADD CONSTRAINT provider_usage_gateway_build_check
        CHECK (gateway_build IS NULL OR gateway_build ~ '^[0-9a-f]{16}$'),
    ADD CONSTRAINT provider_usage_gateway_boot_id_check
        CHECK (gateway_boot_id IS NULL OR gateway_boot_id ~ '^[0-9a-f]{16}$');

CREATE INDEX provider_usage_model_family_time_idx
    ON provider_usage_events (provider, model_family, occurred_at DESC);
CREATE INDEX provider_usage_role_time_idx
    ON provider_usage_events (provider, agent, request_role, occurred_at DESC);
CREATE INDEX provider_usage_session_time_idx
    ON provider_usage_events (provider, account_fingerprint, agent, session_fingerprint, occurred_at DESC)
    WHERE session_fingerprint IS NOT NULL;
