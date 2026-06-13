-- A "system" session is machine-driven and view-only: visible/watchable in the
-- UI, but the composer is hidden and user turns are rejected — only the backend
-- wake endpoint drives it. Used by the mnemosyne memory janitor. Persisted so a
-- system session survives a daemon restart. Mirrors 0008 (awaiting_user/done).
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS system boolean NOT NULL DEFAULT false;
