-- One local, atomic resolution per original operation. No remote replay grant.
-- Retain both the original intent and the newly confirming actor's evidence.
CREATE TABLE plugin_uninstall_resolutions (
    operation_id TEXT PRIMARY KEY REFERENCES plugin_uninstall_operations(operation_id) ON DELETE RESTRICT,
    resolution_id TEXT NOT NULL UNIQUE,
    intent TEXT NOT NULL CHECK (length(CAST(intent AS BLOB)) <= 4096),
    intent_sha256 TEXT NOT NULL,
    resolved_at_ms INTEGER NOT NULL
);
