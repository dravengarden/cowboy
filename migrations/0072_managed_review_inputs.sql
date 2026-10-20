-- Only explicit snapshot-bound reviews participate. The original call keeps
-- owning model execution; duplicates observe it across Controller recovery.
CREATE TABLE managed_review_inputs (
    parent_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    input_identity TEXT NOT NULL,
    call_id TEXT NOT NULL REFERENCES managed_agent_calls(call_id) ON DELETE CASCADE,
    PRIMARY KEY (parent_session_id, input_identity)
);
