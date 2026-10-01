-- Independent execution identity. NULL retains the original runtime-local
-- behavior. Reader support does not enable remote session creation.
ALTER TABLE sessions ADD COLUMN execution_binding TEXT
    CHECK (execution_binding IS NULL OR json_valid(execution_binding));
