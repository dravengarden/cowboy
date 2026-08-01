-- Preserve Machine presence across a short Cowboy controller restart.

ALTER TABLE machines
    DROP CONSTRAINT IF EXISTS machines_status_check;

ALTER TABLE machines
    ADD CONSTRAINT machines_status_check
    CHECK (status IN ('online', 'reconnecting', 'offline', 'updating', 'degraded'));

ALTER TABLE machines
    ADD COLUMN IF NOT EXISTS reconnect_deadline_at timestamptz;
