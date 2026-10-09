-- Child-call ownership is durable before any native launch is dispatched.
CREATE TABLE managed_agent_calls (
    admission_order BIGSERIAL PRIMARY KEY,
    call_id TEXT NOT NULL UNIQUE,
    parent_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    request_id TEXT NOT NULL,
    request_document TEXT NOT NULL,
    child_session_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('queued', 'starting', 'running', 'waiting_input', 'stopping', 'completed', 'failed', 'cancelled')),
    revision BIGINT NOT NULL CHECK (revision > 0),
    document TEXT NOT NULL,
    document_sha256 TEXT NOT NULL,
    UNIQUE (parent_session_id, request_id)
);
CREATE UNIQUE INDEX managed_agent_calls_active_child ON managed_agent_calls (child_session_id)
    WHERE state IN ('queued', 'starting', 'running', 'waiting_input', 'stopping');
CREATE INDEX managed_agent_calls_parent_order ON managed_agent_calls (parent_session_id, admission_order DESC);
CREATE INDEX managed_agent_calls_child_order ON managed_agent_calls (child_session_id, admission_order DESC);
