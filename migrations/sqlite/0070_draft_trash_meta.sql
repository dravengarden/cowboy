-- Rollback-safe Trash metadata (timestamps, session-draft origin, raw ACP blocks)
-- and permanent-purge tombstones (monotonic purge revision + replay protection).
-- draft_documents.value and draft_documents.metadata stay strictly on the legacy JSON schema.
CREATE TABLE draft_document_trash_meta (
    owner_user_id TEXT NOT NULL,
    document_id TEXT NOT NULL,
    trashed_at_ms BIGINT NOT NULL,
    source_session_id TEXT,
    raw_content_json TEXT,
    PRIMARY KEY (owner_user_id, document_id)
);

CREATE TABLE draft_document_tombstones (
    owner_user_id TEXT NOT NULL,
    document_id TEXT NOT NULL,
    revision BIGINT NOT NULL,
    purged_at_ms BIGINT NOT NULL,
    PRIMARY KEY (owner_user_id, document_id)
);
