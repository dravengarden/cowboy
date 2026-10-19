-- Metadata only: native credentials remain in protected authentication storage.
ALTER TABLE sessions ADD COLUMN provider_account_id TEXT NOT NULL DEFAULT 'default';
CREATE TABLE provider_account_registries (
    owner_user_id TEXT PRIMARY KEY,
    revision BIGINT NOT NULL DEFAULT 0 CHECK (revision >= 0),
    value TEXT NOT NULL
);
CREATE TABLE provider_account_operations (
    owner_user_id TEXT NOT NULL,
    operation_id TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    result TEXT NOT NULL,
    PRIMARY KEY (owner_user_id, operation_id)
);
