-- Atomic binding-commit receipt, not Machine readmission or completion.
CREATE TABLE provider_session_transition_commits (
    owner_user_id TEXT NOT NULL,
    operation_id TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (owner_user_id, operation_id),
    FOREIGN KEY (owner_user_id, operation_id)
        REFERENCES provider_session_transitions(owner_user_id, operation_id)
);
