-- Additive reader bridge: existing intent bytes, phases and indexes stay intact.
ALTER TABLE plugin_install_operations ADD COLUMN machine_receipt TEXT
    CHECK (machine_receipt IS NULL OR length(CAST(machine_receipt AS BLOB)) <= 8192);
ALTER TABLE plugin_install_operations ADD COLUMN machine_receipt_sha256 TEXT
    CHECK (
        (machine_receipt IS NULL AND machine_receipt_sha256 IS NULL)
        OR (machine_receipt IS NOT NULL AND machine_receipt_sha256 IS NOT NULL
            AND length(machine_receipt_sha256) = 64)
    );
