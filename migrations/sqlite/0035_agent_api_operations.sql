-- Same receipt contract as PostgreSQL; pending effects are never replayed.
CREATE TABLE agent_api_operations (
    owner_user_id TEXT NOT NULL,
    parent_session_id TEXT NOT NULL,
    operation_id TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    result TEXT,
    created_at_ms BIGINT NOT NULL,
    PRIMARY KEY (owner_user_id, parent_session_id, operation_id)
);
