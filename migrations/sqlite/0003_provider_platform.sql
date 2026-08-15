ALTER TABLE machines ADD COLUMN encryption_public_key TEXT;

ALTER TABLE sessions ADD COLUMN provider_version TEXT NOT NULL DEFAULT '';
ALTER TABLE sessions ADD COLUMN provider_generation_digest TEXT NOT NULL DEFAULT '';
ALTER TABLE sessions ADD COLUMN provider_auth_generation INTEGER;
ALTER TABLE sessions ADD COLUMN provider_behavior TEXT;
ALTER TABLE sessions ADD COLUMN purge_after_at_ms INTEGER;

CREATE INDEX sessions_provider_generation_idx
  ON sessions(machine_id, provider, provider_generation_digest)
  WHERE deleted_at_ms IS NULL;

CREATE INDEX sessions_purge_after_idx
  ON sessions(purge_after_at_ms)
  WHERE deleted_at_ms IS NOT NULL AND purge_after_at_ms IS NOT NULL;
