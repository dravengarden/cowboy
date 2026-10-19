-- Historical process-stop evidence, separate from immutable revocation plans.
CREATE TABLE provider_account_stops (
    owner_user_id TEXT NOT NULL,
    operation_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (owner_user_id, operation_id, session_id),
    FOREIGN KEY (owner_user_id, operation_id)
        REFERENCES provider_account_operations(owner_user_id, operation_id)
);
