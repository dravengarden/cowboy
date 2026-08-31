-- SQLite cannot add a derived NOT NULL column directly. Application writes
-- always populate this field; the backfill preserves existing sessions.

ALTER TABLE user_sessions
    ADD COLUMN primary_authenticated_at_ms INTEGER;

UPDATE user_sessions
SET primary_authenticated_at_ms = created_at_ms
WHERE primary_authenticated_at_ms IS NULL;
