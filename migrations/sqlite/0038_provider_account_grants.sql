-- Internal login promotion journal; contains identities and receipts, never credentials.
CREATE TABLE provider_account_grants (
    owner_user_id TEXT NOT NULL,
    operation_id TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (owner_user_id, operation_id)
);
