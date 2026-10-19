-- Dispatch fences, not credentials or proof that native login succeeded.
CREATE TABLE provider_login_operations (
    owner_user_id TEXT NOT NULL,
    operation_id TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    request_id TEXT NOT NULL UNIQUE,
    value TEXT NOT NULL,
    PRIMARY KEY (owner_user_id, operation_id)
);
