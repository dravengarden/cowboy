-- Independent, principal-owned writing. Neither documents nor folders bind a Machine.
CREATE TABLE draft_document_owners (
    owner_user_id TEXT PRIMARY KEY,
    revision BIGINT NOT NULL DEFAULT 0
);
CREATE TABLE draft_documents (
    owner_user_id TEXT NOT NULL,
    id TEXT NOT NULL,
    value TEXT NOT NULL,
    metadata TEXT NOT NULL,
    PRIMARY KEY (owner_user_id, id)
);
CREATE TABLE draft_document_operations (
    owner_user_id TEXT NOT NULL,
    operation_id TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    document_id TEXT NOT NULL,
    PRIMARY KEY (owner_user_id, operation_id)
);
CREATE TABLE draft_document_history (
    owner_user_id TEXT NOT NULL,
    document_id TEXT NOT NULL,
    revision BIGINT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (owner_user_id, document_id, revision)
);
