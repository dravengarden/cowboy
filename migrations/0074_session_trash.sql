-- Sessions Trash (docs/sessions-folders.md#trash). A trashed session or folder
-- keeps every row and its placement, so restore returns it to where it was.
-- The Controller permanently deletes it once the owner's retention elapses.
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS trashed_at_ms BIGINT;
ALTER TABLE session_folders ADD COLUMN IF NOT EXISTS trashed_at_ms BIGINT;
