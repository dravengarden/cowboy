-- LLM inference providers (design: tasks/active/confirm-detect-skills).
--
-- Skills' L2 judge calls an external LLM ("inference provider", e.g. DeepSeek).
-- Two small per-provider tables, keyed by the provider id:
--
-- `inference_config`: the non-secret settings — the selected `model` and a
--   `params` JSONB bag (temperature, max_tokens, … — JSONB so new knobs never
--   need a migration). One row per provider.
-- `inference_secrets`: the API key, split out so it's easy to keep out of every
--   non-secret read path (the UI only ever asks "is a key set?", never the key).
--   Plaintext for v1 (LAN / single-user); never logged, never echoed to the web.

CREATE TABLE IF NOT EXISTS inference_config (
  provider   text PRIMARY KEY,
  model      text NOT NULL,
  params     jsonb NOT NULL DEFAULT '{}'::jsonb,
  updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS inference_secrets (
  provider   text PRIMARY KEY,
  api_key    text NOT NULL,
  updated_at timestamptz NOT NULL DEFAULT now()
);
