-- SQLite mirror of 0090_provider_account_reset_schedules.sql.
-- One owner-scheduled banked reset per named Provider account. A row is the
-- one-shot authority to spend the account's next reset at `fire_at_ms`: the
-- timer deletes it (compare on `idempotency_key`) before anything is sent,
-- so it fires at most once. `authority` binds the schedule to the account's
-- sign-in at scheduling time; any change of it withdraws the row unspent.
-- No foreign keys: accounts live in the owner's serialized registry, and a
-- removed account or owner withdraws the row when it comes due.
CREATE TABLE provider_account_reset_schedules (
    owner_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    fire_at_ms BIGINT NOT NULL,
    next_attempt_at_ms BIGINT NOT NULL,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    idempotency_key TEXT NOT NULL,
    authority TEXT NOT NULL,
    PRIMARY KEY (owner_id, account_id)
);

CREATE INDEX provider_account_reset_schedules_due_idx
    ON provider_account_reset_schedules (next_attempt_at_ms);
