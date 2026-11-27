-- Rollback-safe Trash metadata (revision-bound timestamps, session-draft origin,
-- raw ACP blocks, DeliveryOrigin, recovery classification), permanent-purge
-- tombstones / purged-operation receipts, and server-owned folder-only Trash undo receipts.
-- draft_documents.value and draft_documents.metadata stay strictly on the legacy JSON schema.
CREATE TABLE draft_document_trash_meta (
    owner_user_id TEXT NOT NULL,
    document_id TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 0,
    trashed_at_ms INTEGER NOT NULL,
    source_session_id TEXT,
    raw_content_json TEXT,
    source_origin_json TEXT,
    source_is_recovery INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (owner_user_id, document_id)
);

CREATE TABLE draft_document_tombstones (
    owner_user_id TEXT NOT NULL,
    document_id TEXT NOT NULL,
    revision INTEGER NOT NULL,
    purged_at_ms INTEGER NOT NULL,
    restore_op_id TEXT,
    restored_row_id TEXT,
    restore_session_id TEXT,
    restore_expected_revision INTEGER,
    restore_expected_trashed_at_ms INTEGER,
    PRIMARY KEY (owner_user_id, document_id)
);

CREATE TABLE draft_document_purged_ops (
    owner_user_id TEXT NOT NULL,
    operation_id TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    document_id TEXT NOT NULL,
    purged_revision INTEGER NOT NULL,
    PRIMARY KEY (owner_user_id, operation_id)
);

CREATE TABLE folder_trash_only_receipts (
    receipt_id TEXT PRIMARY KEY,
    owner_user_id TEXT,
    folder_id TEXT NOT NULL,
    trashed_at_ms INTEGER NOT NULL,
    payload_json TEXT NOT NULL,
    undone_at_ms INTEGER
);
