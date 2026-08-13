ALTER TABLE scheduled_provider_actions
    DROP CONSTRAINT IF EXISTS scheduled_provider_actions_provider_check;

ALTER TABLE scheduled_provider_actions
    ADD CONSTRAINT scheduled_provider_actions_provider_check
    CHECK (provider IN ('codex', 'xai'));

ALTER TABLE provider_action_logs
    DROP CONSTRAINT IF EXISTS provider_action_logs_provider_check;

ALTER TABLE provider_action_logs
    ADD CONSTRAINT provider_action_logs_provider_check
    CHECK (provider IN ('codex', 'xai'));
