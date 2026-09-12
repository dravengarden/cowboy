-- A fixed Service export slot. The empty reader bridge adopts no legacy policy.
-- Head, intent and complete Machine evidence are a single bounded atomic row.
CREATE TABLE telemetry_binding_journal (
    slot TEXT PRIMARY KEY NOT NULL CHECK (slot = 'telemetry'),
    document TEXT NOT NULL CHECK (length(CAST(document AS BLOB)) <= 4194304),
    document_sha256 TEXT NOT NULL CHECK (length(document_sha256) = 64)
);
