-- Core installation evidence, separate from uninstall impact and credentials.
CREATE TABLE plugin_install_operations (
    operation_id TEXT PRIMARY KEY,
    service_id TEXT NOT NULL,
    machine_id TEXT NOT NULL,
    plugin_id TEXT NOT NULL,
    intent TEXT NOT NULL CHECK (octet_length(intent) <= 4096),
    intent_sha256 TEXT NOT NULL,
    phase TEXT NOT NULL CHECK (phase IN (
        'prepared', 'syncing_authentication', 'installing', 'machine_acknowledged',
        'completed', 'authentication_pending', 'aborted', 'needs_attention'
    )),
    problem TEXT,
    attention_from TEXT,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL
);
CREATE UNIQUE INDEX plugin_install_open_slot
    ON plugin_install_operations (machine_id, plugin_id)
    WHERE phase NOT IN ('completed', 'authentication_pending', 'aborted');
CREATE INDEX plugin_install_target_history
    ON plugin_install_operations (service_id, machine_id, plugin_id, created_at_ms DESC);
