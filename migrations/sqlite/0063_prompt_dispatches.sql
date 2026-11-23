-- Authoritative Controller-side prompt and steer dispatch origins (C-auth).
-- Records the computed dispatch origin and initiating trigger for every
-- prompt/steer handed off by the Controller, keyed by (session_id, cmid).
CREATE TABLE prompt_dispatches (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    cmid TEXT NOT NULL,
    origin TEXT NOT NULL,
    trigger_origin TEXT NOT NULL,
    dispatched_at_ms BIGINT NOT NULL,
    PRIMARY KEY (session_id, cmid)
);

CREATE INDEX prompt_dispatches_session_at_idx
    ON prompt_dispatches (session_id, dispatched_at_ms DESC);
