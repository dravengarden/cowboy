-- Track exact per-dispatch non-delivery retirement atomically with pending
-- draft persistence so crashed/parked recovery drafts never rearm
-- UndeliveredPrompt on restart without relying on wall-clock comparisons.
ALTER TABLE prompt_dispatches
    ADD COLUMN undelivered_retired INTEGER NOT NULL DEFAULT 0;
