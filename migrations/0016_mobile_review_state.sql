-- Mobile-only code-review workspace state.
--
-- This belongs to the Cowboy session rather than a browser installation:
-- iPhone/iPad clients share open source tabs, the active source, review mode,
-- and reviewed revisions. Desktop deliberately does not consume this state.

ALTER TABLE sessions
  ADD COLUMN IF NOT EXISTS mobile_review_state jsonb NOT NULL DEFAULT
    '{"mode":"git","tabs":[],"progress":{}}'::jsonb;
