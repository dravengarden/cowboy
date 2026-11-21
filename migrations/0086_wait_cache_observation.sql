-- Bounded, read-only reconciliation against the authoritative Call ledger and
-- the receiver's durable prompt echo. These indexes own no notice state.
ALTER TABLE managed_agent_calls ADD COLUMN notice_finished_at_ms BIGINT;
ALTER TABLE managed_agent_calls ADD COLUMN notice_parent BOOLEAN NOT NULL DEFAULT FALSE;

-- PostgreSQL cannot turn an escaped NUL into text. Mask it in this one-time
-- projection so unrelated result content cannot prevent backfilling columns.
-- The checksummed document is preserved byte for byte.
WITH legacy AS (
    SELECT call_id, replace(document, $nul$\u0000$nul$, $replacement$\uFFFD$replacement$)::json AS metadata
    FROM managed_agent_calls WHERE state IN ('completed', 'failed', 'cancelled')
)
UPDATE managed_agent_calls AS c
SET notice_finished_at_ms = COALESCE(
        (legacy.metadata ->> 'finished_at_ms')::bigint,
        (legacy.metadata ->> 'updated_at_ms')::bigint),
    notice_parent = TRUE
FROM legacy
WHERE c.call_id = legacy.call_id AND legacy.metadata #>> '{labels,notify}' = 'parent';

CREATE INDEX managed_call_notice_finished_idx
    ON managed_agent_calls (notice_finished_at_ms, call_id)
    WHERE notice_parent AND notice_finished_at_ms IS NOT NULL;

CREATE INDEX events_call_notice_echo_idx
    ON events (session_id, (payload ->> 'cmid'))
    WHERE payload ->> 'cmid' LIKE 'call-notice:%'
      AND payload ->> 'kind' = 'update'
      AND payload #>> '{update,sessionUpdate}' = 'user_message_chunk';

CREATE INDEX events_context_boundary_idx ON events (session_id, seq DESC)
    WHERE payload->'update'->>'sessionUpdate' IN ('context_cleared', 'context_rolled_over');
