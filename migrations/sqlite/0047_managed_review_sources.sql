-- Same durable per-call observation contract as PostgreSQL.
CREATE TABLE managed_review_sources (
    parent_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    call_id TEXT NOT NULL REFERENCES managed_agent_calls(call_id) ON DELETE CASCADE,
    source_call_id TEXT NOT NULL REFERENCES managed_agent_calls(call_id) ON DELETE CASCADE,
    PRIMARY KEY (parent_session_id, call_id)
);
