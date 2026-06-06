-- Manual session ordering (drag-to-arrange).
--
-- The session list was ordered by created_at. To let the user drag sessions
-- into a custom order (synced across terminals like queue/drafts), record an
-- explicit position per session. NULL until first reordered; load_all sorts by
-- `position ASC NULLS LAST, created_at ASC`, so never-reordered rows keep their
-- creation order and a partial reorder degrades gracefully. No backfill needed.
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS position double precision;
