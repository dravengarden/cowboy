-- One-shot reset schedules move to in flight before any Provider call: a
-- crash, cancel or replacement during an attempt can never make the same
-- schedule runnable again (an interrupted attempt is recorded as unknown).
-- A named account's schedule is bound to the authority that created it and
-- is withdrawn when that authority changes.
ALTER TABLE scheduled_provider_actions ADD COLUMN in_flight_at_ms INTEGER;
ALTER TABLE scheduled_provider_actions ADD COLUMN authority TEXT
    CHECK (authority IS NULL OR length(authority) BETWEEN 1 AND 128);
