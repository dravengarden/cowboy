-- Durable Machine browser placements: where an account's browser profile
-- lives, one per owner, account and Machine. Captured into the account's
-- revocation plan; never evidence that an instance is running.
CREATE TABLE browser_placements (
    owner_user_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    machine_id TEXT NOT NULL,
    instance_id TEXT NOT NULL,
    profile_revision BIGINT NOT NULL DEFAULT 1,
    last_open_ms BIGINT NOT NULL,
    PRIMARY KEY (owner_user_id, account_id, machine_id)
);
