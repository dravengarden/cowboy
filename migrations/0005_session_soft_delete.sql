-- Soft-delete: a user "delete" now marks `deleted_at` instead of dropping the
-- row, and a background sweeper hard-deletes (cascade → events) rows whose
-- deleted_at is older than the retention window (3 days). This bounds the
-- storage a deleted session holds while leaving a recovery window, and `load_all`
-- skips soft-deleted rows so they vanish from the UI immediately.
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS deleted_at timestamptz;

-- Partial index so the sweeper's `WHERE deleted_at < ...` scan stays cheap.
CREATE INDEX IF NOT EXISTS sessions_deleted_at_idx ON sessions (deleted_at)
  WHERE deleted_at IS NOT NULL;
