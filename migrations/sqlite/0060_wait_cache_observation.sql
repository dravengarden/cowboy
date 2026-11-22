-- Same observation indexes as PostgreSQL. No delivery state or policy changes.
ALTER TABLE managed_agent_calls ADD COLUMN notice_finished_at_ms INTEGER;
ALTER TABLE managed_agent_calls ADD COLUMN notice_parent BOOLEAN NOT NULL DEFAULT FALSE;

UPDATE managed_agent_calls
SET notice_finished_at_ms = CAST(COALESCE(
        json_extract(document, '$.finished_at_ms'), json_extract(document, '$.updated_at_ms')) AS INTEGER),
    notice_parent = TRUE
WHERE state IN ('completed', 'failed', 'cancelled')
  AND json_extract(document, '$.labels.notify') = 'parent';

CREATE INDEX managed_call_notice_finished_idx
    ON managed_agent_calls (notice_finished_at_ms, call_id)
    WHERE notice_parent AND notice_finished_at_ms IS NOT NULL;

CREATE INDEX events_call_notice_echo_idx
    ON events (session_id, json_extract(payload, '$.cmid'))
    WHERE json_extract(payload, '$.cmid') LIKE 'call-notice:%'
      AND json_extract(payload, '$.kind') = 'update'
      AND json_extract(payload, '$.update.sessionUpdate') = 'user_message_chunk';

CREATE INDEX events_context_boundary_idx ON events (session_id, seq DESC)
    WHERE json_extract(payload, '$.update.sessionUpdate') IN ('context_cleared', 'context_rolled_over');
