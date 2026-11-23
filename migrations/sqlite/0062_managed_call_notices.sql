-- Durable completion-notification delivery ledger for opted-in Managed Calls
-- (Phase 2A of wait-cache observability). One immutable completion_id per
-- terminal Call with explicit enqueued / accepted / consumed / unknown states.
CREATE TABLE managed_call_notices (
    completion_id TEXT PRIMARY KEY,
    call_id TEXT NOT NULL UNIQUE REFERENCES managed_agent_calls(call_id) ON DELETE CASCADE,
    parent_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    state TEXT NOT NULL CHECK (state IN ('enqueued', 'accepted', 'consumed', 'unknown')),
    dispatch_started_at_ms BIGINT,
    dispatch_command_id TEXT,
    dispatch_worker_epoch TEXT,
    echo_seq BIGINT,
    reason TEXT,
    last_undelivered_key TEXT,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL
);

CREATE INDEX managed_call_notices_parent_idx
    ON managed_call_notices (parent_session_id, updated_at_ms DESC);

CREATE INDEX managed_call_notices_unsettled_idx
    ON managed_call_notices (created_at_ms ASC, completion_id ASC)
    WHERE state IN ('enqueued', 'accepted')
       OR (state = 'unknown' AND reason IN ('worker_stopped', 'turn_unconfirmed') AND echo_seq IS NOT NULL);

-- Typed opt-in flag populated at admission so transcript clear never parses
-- request_document JSON at runtime.
ALTER TABLE managed_agent_calls
    ADD COLUMN request_notify_parent BOOLEAN NOT NULL DEFAULT FALSE;

UPDATE managed_agent_calls
SET request_notify_parent = TRUE
WHERE notice_parent = TRUE
   OR COALESCE(json_extract(request_document, '$.labels.notify'), '') = 'parent';
