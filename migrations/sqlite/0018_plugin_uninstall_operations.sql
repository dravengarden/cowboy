-- Core-owned recovery evidence. Neither Plugin uninstall nor session GC owns it.
CREATE TABLE plugin_uninstall_operations (
    operation_id TEXT PRIMARY KEY,
    service_id TEXT NOT NULL,
    machine_id TEXT NOT NULL,
    plugin_id TEXT NOT NULL,
    intent TEXT NOT NULL CHECK (length(CAST(intent AS BLOB)) <= 262144),
    intent_sha256 TEXT NOT NULL,
    phase TEXT NOT NULL CHECK (phase IN (
        'prepared', 'stopping_sessions', 'uninstalling', 'machine_uninstalled',
        'restoring_machine', 'restoring_sessions', 'completed', 'compensated',
        'aborted', 'needs_attention'
    )),
    problem TEXT,
    cause TEXT,
    attention_from TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
CREATE UNIQUE INDEX plugin_uninstall_open_slot
    ON plugin_uninstall_operations (machine_id, plugin_id)
    WHERE phase NOT IN ('completed', 'compensated', 'aborted');
CREATE INDEX plugin_uninstall_target_history
    ON plugin_uninstall_operations (machine_id, plugin_id, created_at_ms DESC);
