-- Admission is durable before an Agent API effect is attempted. A pending row
-- after a crash is uncertain, never permission to replay the effect.
CREATE TABLE agent_api_operations (
    owner_user_id TEXT NOT NULL,
    parent_session_id TEXT NOT NULL,
    operation_id TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    result TEXT,
    created_at_ms BIGINT NOT NULL,
    PRIMARY KEY (owner_user_id, parent_session_id, operation_id)
);
