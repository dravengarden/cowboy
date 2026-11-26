-- Durable fence for Machine browser navigations whose terminal outcome is not
-- yet confirmed (including across Controller restarts and Machine disconnects).
-- Share membership mutations (`SetBrowserShare`, `DeleteBrowserShare`) and
-- share resets wait until each row settles, its Machine/placement is revoked,
-- or its bounded TTL expires. Account revocation (`BeginRevocation`) is an
-- explicit non-blocking exception that leaves shares immediately while keeping
-- the account's family binding until a confirmed reset.
CREATE TABLE browser_pending_navigations (
    request_id TEXT PRIMARY KEY,
    owner_user_id TEXT NOT NULL,
    browser_key TEXT NOT NULL,
    machine_id TEXT NOT NULL,
    instance_id TEXT NOT NULL,
    incarnation BIGINT NOT NULL,
    observed_at_ms BIGINT NOT NULL,
    started_at_ms BIGINT NOT NULL
);
CREATE INDEX idx_browser_pending_navigations_owner_key
    ON browser_pending_navigations (owner_user_id, browser_key);
CREATE INDEX idx_browser_pending_navigations_machine
    ON browser_pending_navigations (machine_id);
