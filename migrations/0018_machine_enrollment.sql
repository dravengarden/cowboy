-- Public-key enrollment for outbound Machine connections.

ALTER TABLE machines
    ADD COLUMN IF NOT EXISTS public_key text,
    ADD COLUMN IF NOT EXISTS enrolled_at timestamptz,
    ADD COLUMN IF NOT EXISTS revoked_at timestamptz,
    ADD COLUMN IF NOT EXISTS connection_epoch text;

CREATE TABLE IF NOT EXISTS machine_enrollment_tokens (
    token_hash text PRIMARY KEY,
    machine_id text NOT NULL UNIQUE,
    display_name text NOT NULL,
    expires_at timestamptz NOT NULL,
    used_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS machine_enrollment_expiry_idx
    ON machine_enrollment_tokens(expires_at)
    WHERE used_at IS NULL;
