-- Task notifications (docs/task-notifications.md). Intents are durable
-- occurrences, deliveries freeze their audience, subscriptions belong to an
-- account, and acknowledgements live as long as their session. Older
-- Controllers ignore these tables.
CREATE TABLE IF NOT EXISTS notification_intents (
    dedupe_key TEXT PRIMARY KEY,
    owner_user_id TEXT NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    category TEXT NOT NULL CHECK (category IN ('input', 'failure', 'completion')),
    destination TEXT NOT NULL DEFAULT '',
    marker TEXT NOT NULL DEFAULT '',
    due_at_ms BIGINT NOT NULL,
    expires_at_ms BIGINT NOT NULL,
    state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending', 'resolved', 'expired')),
    expanded_at_ms BIGINT,
    created_at_ms BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS notification_intents_due
    ON notification_intents (due_at_ms) WHERE state = 'pending' AND expanded_at_ms IS NULL;
CREATE INDEX IF NOT EXISTS notification_intents_created ON notification_intents (created_at_ms);

CREATE TABLE IF NOT EXISTS notification_deliveries (
    dedupe_key TEXT NOT NULL REFERENCES notification_intents(dedupe_key) ON DELETE CASCADE,
    endpoint_sha256 TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending', 'sent', 'failed', 'dropped')),
    attempts BIGINT NOT NULL DEFAULT 0,
    next_attempt_at_ms BIGINT NOT NULL,
    PRIMARY KEY (dedupe_key, endpoint_sha256)
);
CREATE INDEX IF NOT EXISTS notification_deliveries_due
    ON notification_deliveries (next_attempt_at_ms) WHERE state = 'pending';

CREATE TABLE IF NOT EXISTS web_push_subscriptions (
    endpoint_sha256 TEXT PRIMARY KEY,
    endpoint TEXT NOT NULL,
    owner_user_id TEXT NOT NULL,
    enrolled_session_id TEXT,
    p256dh TEXT NOT NULL,
    auth TEXT NOT NULL,
    preferences TEXT NOT NULL,
    updated_at_ms BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS web_push_subscriptions_owner ON web_push_subscriptions (owner_user_id);

CREATE TABLE IF NOT EXISTS task_acknowledgements (
    owner_user_id TEXT NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    task_key TEXT NOT NULL CHECK (length(task_key) BETWEEN 1 AND 300),
    marker TEXT NOT NULL CHECK (length(marker) BETWEEN 1 AND 64),
    acknowledged_at_ms BIGINT NOT NULL,
    PRIMARY KEY (owner_user_id, session_id, task_key, marker)
);
