-- Last context-window usage the agent reported for a session (ACP
-- `usage_update`). A dormant session has no worker
-- to re-report it, so the Controller restores this snapshot and the
-- composer keeps showing the last recorded reading until a worker reports a
-- fresh one. Clear resets it to NULL. Older Controllers ignore the column.
ALTER TABLE sessions ADD COLUMN last_usage TEXT
    CHECK (last_usage IS NULL OR json_valid(last_usage));
