-- Account controls of named-only Providers
-- (docs/antigravity-real-accounts-design.md, "Account controls"). One durable
-- state per account; anything but 'ok' parks the account's queues.
CREATE TABLE provider_account_controls (
    account_id TEXT PRIMARY KEY CHECK (length(account_id) BETWEEN 1 AND 64),
    owner_user_id TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('ok', 'verification_required',
        'location_unsupported', 'rate_limited', 'suspended', 'blocked',
        'unknown_refusal', 'needs_attention')),
    reason TEXT NOT NULL CHECK (length(reason) <= 256),
    until_ms INTEGER,
    updated_at_ms INTEGER NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1)
) WITHOUT ROWID;

-- The account's single active turn (R1-8, R2-11, R3-9): taken atomically
-- before dispatch, 'may_have_dispatched' before the prompt is enqueued.
CREATE TABLE provider_account_turns (
    account_id TEXT PRIMARY KEY CHECK (length(account_id) BETWEEN 1 AND 64),
    owner_user_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    turn_id TEXT NOT NULL CHECK (length(turn_id) BETWEEN 1 AND 256),
    worker_epoch TEXT NOT NULL,
    after_seq INTEGER NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('reserved', 'may_have_dispatched')),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
) WITHOUT ROWID;

-- Machine fences the Controller owes an account's daemon; one current
-- operation per Machine, retried with the same operation ID until confirmed.
CREATE TABLE provider_account_fences (
    account_id TEXT NOT NULL CHECK (length(account_id) BETWEEN 1 AND 64),
    machine_id TEXT NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('stop', 'admit')),
    operation_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('pending', 'confirmed')),
    detail TEXT NOT NULL CHECK (length(detail) <= 512),
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY (account_id, machine_id)
) WITHOUT ROWID;
