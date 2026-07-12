-- Auto-resume interrupted turns (design: tasks/active/session-auto-resume).
--
-- `sessions.auto_resume`: per-session OVERRIDE of the global default.
--   NULL  = inherit the global default (settings key 'session.autoResume.default')
--   true  = always auto-continue an interrupted turn for this session
--   false = never (explicit opt-out, e.g. when the global default is on)
-- Effective = COALESCE(auto_resume, global_default). Existing rows default to
-- NULL = inherit, so behavior is unchanged until a default/override is set.
--
-- `settings`: a small global key-value store (its first users are the
--   auto-resume default flag + the continuation-message template). `value` is
--   JSONB so new settings never need another migration.

ALTER TABLE sessions ADD COLUMN IF NOT EXISTS auto_resume boolean;

CREATE TABLE IF NOT EXISTS settings (
  key        text PRIMARY KEY,
  value      jsonb NOT NULL,
  updated_at timestamptz NOT NULL DEFAULT now()
);
