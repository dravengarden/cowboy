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
