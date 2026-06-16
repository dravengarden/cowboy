-- One pending ScheduleWakeup per session, so an agent-armed wakeup survives a
-- daemon restart and still fires (the in-memory scheduler is re-armed from this
-- on startup). ON DELETE CASCADE auto-clears a deleted session's wakeup.
CREATE TABLE IF NOT EXISTS scheduled_wakeups (
    session_id  TEXT   PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
    fire_at_ms  BIGINT NOT NULL,
    prompt      TEXT   NOT NULL
);
