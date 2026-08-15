-- Product-level Provider generations, Machine replica identity, and absolute
-- uninstall retention. Service auth state remains in its encrypted filesystem
-- vault so there is only one durable authority for each generation.

ALTER TABLE machines
  ADD COLUMN IF NOT EXISTS encryption_public_key text;

ALTER TABLE sessions
  ADD COLUMN IF NOT EXISTS provider_version text NOT NULL DEFAULT '',
  ADD COLUMN IF NOT EXISTS provider_generation_digest text NOT NULL DEFAULT '',
  ADD COLUMN IF NOT EXISTS provider_auth_generation bigint,
  ADD COLUMN IF NOT EXISTS provider_behavior jsonb,
  ADD COLUMN IF NOT EXISTS purge_after_at timestamptz;

CREATE INDEX IF NOT EXISTS sessions_provider_generation_idx
  ON sessions (machine_id, provider, provider_generation_digest)
  WHERE deleted_at IS NULL;

CREATE INDEX IF NOT EXISTS sessions_purge_after_idx
  ON sessions (purge_after_at)
  WHERE deleted_at IS NOT NULL AND purge_after_at IS NOT NULL;
