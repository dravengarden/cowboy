-- Server-authoritative queue + drafts (cross-terminal sync).
--
-- The send-queue (prompts waiting for the current turn to finish) and parked
-- drafts used to live in each browser's localStorage, so they never synced
-- across devices/terminals. They are now owned by the daemon and broadcast to
-- every client. Persist both per session so staged messages survive a daemon
-- restart — matching the durability the old localStorage gave them.
--
-- Each is a JSONB array of { id, text, content } (content = the ACP content
-- blocks, empty for a plain-text message). Default '[]' means no backfill: every
-- existing session simply starts with empty lists.
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS queue jsonb NOT NULL DEFAULT '[]'::jsonb;
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS drafts jsonb NOT NULL DEFAULT '[]'::jsonb;
