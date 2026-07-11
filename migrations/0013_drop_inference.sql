-- Cowboy now uses the host Codex app-server for confirm classification. Remove
-- the obsolete external-provider configuration and its stored API secrets.
DROP TABLE IF EXISTS inference_secrets;
DROP TABLE IF EXISTS inference_config;
