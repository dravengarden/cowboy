-- Provider reset capabilities are declared by signed Plugin host contracts.
-- Keep persistence generic so a new conforming Provider does not require a
-- Controller release merely to name its reset lane.

ALTER TABLE scheduled_provider_actions
    DROP CONSTRAINT IF EXISTS scheduled_provider_actions_provider_check;

ALTER TABLE scheduled_provider_actions
    ADD CONSTRAINT scheduled_provider_actions_provider_check CHECK (
        char_length(provider) BETWEEN 1 AND 64
        AND provider ~ '^[a-z0-9]+(-[a-z0-9]+)*$'
    );

ALTER TABLE provider_action_logs
    DROP CONSTRAINT IF EXISTS provider_action_logs_provider_check;

ALTER TABLE provider_action_logs
    ADD CONSTRAINT provider_action_logs_provider_check CHECK (
        char_length(provider) BETWEEN 1 AND 64
        AND provider ~ '^[a-z0-9]+(-[a-z0-9]+)*$'
    );
