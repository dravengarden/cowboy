-- Track exact per-dispatch undelivered retirement and durable per-session
-- ordering so recovery parking and restart hydration never rely on wall-clock comparisons.
ALTER TABLE prompt_dispatches
    ADD COLUMN undelivered_retired INTEGER NOT NULL DEFAULT 0;
ALTER TABLE prompt_dispatches
    ADD COLUMN dispatch_seq INTEGER NOT NULL DEFAULT 0;
