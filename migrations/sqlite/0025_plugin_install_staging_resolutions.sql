-- A fresh Operator may retire one exact activation-free staging failure.
-- The original uncertain receipt stays immutable in plugin_install_operations.
CREATE TABLE plugin_install_staging_resolutions (
    operation_id TEXT PRIMARY KEY REFERENCES plugin_install_operations(operation_id) ON DELETE RESTRICT,
    resolution_id TEXT NOT NULL UNIQUE,
    intent TEXT NOT NULL CHECK (length(CAST(intent AS BLOB)) <= 4096),
    intent_sha256 TEXT NOT NULL CHECK (length(intent_sha256) = 64),
    resolved_at_ms INTEGER NOT NULL CHECK (resolved_at_ms > 0)
);

ALTER TABLE plugin_install_operations ADD COLUMN staging_resolved_at_ms INTEGER
    CHECK (staging_resolved_at_ms IS NULL OR staging_resolved_at_ms > 0);

DROP INDEX plugin_install_open_slot;
CREATE UNIQUE INDEX plugin_install_open_slot
    ON plugin_install_operations (machine_id, plugin_id)
    WHERE phase NOT IN ('completed', 'authentication_pending', 'aborted')
      AND staging_resolved_at_ms IS NULL;
