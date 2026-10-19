-- Exact lookup only; the journal and current account/enrollment remain authority.
CREATE TABLE provider_grant_routes (
    commitment TEXT PRIMARY KEY,
    owner_user_id TEXT NOT NULL,
    operation_id TEXT NOT NULL,
    UNIQUE (owner_user_id, operation_id)
);
