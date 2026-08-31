-- Preserve the original primary-login proof across Passkey cookie rotation.
-- last_seen_at remains the coarse, server-owned human-activity timestamp.

ALTER TABLE user_sessions
    ADD COLUMN primary_authenticated_at timestamptz;

UPDATE user_sessions
SET primary_authenticated_at = created_at
WHERE primary_authenticated_at IS NULL;

ALTER TABLE user_sessions
    ALTER COLUMN primary_authenticated_at SET NOT NULL;
