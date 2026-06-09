-- Persist the confirm-detect turn-end verdict (awaiting_user / done) so a daemon
-- restart doesn't wipe a finished session's state. Previously transient — warm
-- restore hardcoded both to false on the theory "the next turn re-judges" — but a
-- DONE session has no next turn, so its green "Task complete" vanished on every
-- restart. Now durable: the judge's verdict is written here and restored into
-- SessionMeta on startup. Existing rows default to false (cleared), unchanged.
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS awaiting_user boolean NOT NULL DEFAULT false;
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS done boolean NOT NULL DEFAULT false;
