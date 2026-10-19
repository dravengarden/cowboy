-- Migration intent is not process-stop evidence or runtime admission.
CREATE TABLE provider_session_transitions (
    owner_user_id TEXT NOT NULL,
    operation_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    value TEXT NOT NULL,
    finished BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY (owner_user_id, operation_id)
);
CREATE UNIQUE INDEX provider_session_transition_active ON provider_session_transitions(session_id) WHERE NOT finished;
