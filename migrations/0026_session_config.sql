-- Persist the agent-advertised config surface and the user's session-owned
-- choices. `config_options` is the latest display snapshot; `config_preferences`
-- contains only values selected by the user (or seeded defaults), so a provider
-- can refresh its option list without erasing the session's intent.
ALTER TABLE sessions
    ADD COLUMN config_options jsonb,
    ADD COLUMN config_preferences jsonb NOT NULL DEFAULT '{}'::jsonb;

-- Before this migration Cowboy had no durable per-session config choices, so an
-- empty preference object means the session was using the provider default. Make
-- the new OpenAI default apply to those existing sessions as well; subsequent
-- user changes replace these values through the service write path.
UPDATE sessions
SET config_preferences = '{"model":"gpt-5.6-luna","reasoning_effort":"max"}'::jsonb
WHERE provider = 'codex' AND config_preferences = '{}'::jsonb;
