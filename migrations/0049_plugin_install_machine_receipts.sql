-- Additive reader bridge: existing intent bytes, phases and indexes stay intact.
ALTER TABLE plugin_install_operations
    ADD COLUMN machine_receipt TEXT,
    ADD COLUMN machine_receipt_sha256 TEXT,
    ADD CONSTRAINT plugin_install_machine_receipt_pair CHECK (
        (machine_receipt IS NULL AND machine_receipt_sha256 IS NULL)
        OR (machine_receipt IS NOT NULL AND machine_receipt_sha256 IS NOT NULL
            AND octet_length(machine_receipt) <= 8192
            AND length(machine_receipt_sha256) = 64)
    );
