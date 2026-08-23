-- Consolidated PostgreSQL baseline.
--
-- Fresh databases execute the former v1-v33 history in one SQLx transaction.
-- The compatibility ledger rows keep rollback to the immediately preceding
-- controller generation safe; existing v33 databases are marked at v34 by
-- the preflight in Store::migrate and do not execute this file.

-- Former migration: 0001_init.sql

-- Initial schema for cowboy's persistent state.
--
-- Two tables:
--
-- `sessions`: one row per cowboy session. Status is the latest known process
--             state (Starting / Running / Busy / Exited / Crashed); origin is
--             the surface that opened the session (api / web / zed) used by
--             the UI's badge. `next_seq` is the high-water mark of the
--             monotonic per-session counter; it's authoritatively maintained
--             in-memory by `Hub` and mirrored here so a fresh `Hub` after
--             restart can pick up where we left off.
--
-- `events`: the per-session, seq-ordered log of envelopes. `payload` is the
--           `Event` enum (Update / PermissionRequest / PermissionResolved /
--           Lifecycle / TurnEnd) serialized as JSONB so cowboy can roundtrip
--           new variants without migrations.
--
-- ON DELETE CASCADE on events means dropping a session also drops its log —
-- consistent with the in-memory `Hub::delete_session` behaviour.

CREATE TABLE IF NOT EXISTS sessions (
  id         text PRIMARY KEY,
  provider   text NOT NULL,
  cwd        text NOT NULL,
  title      text NOT NULL,
  origin     text NOT NULL,
  status     text NOT NULL,
  next_seq   bigint NOT NULL DEFAULT 0,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS events (
  session_id text   NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  seq        bigint NOT NULL,
  ts         timestamptz NOT NULL DEFAULT now(),
  payload    jsonb  NOT NULL,
  PRIMARY KEY (session_id, seq)
);

-- Used for the snapshot replay on WS connect (`Hub::snapshot`) and for the
-- full-session load on daemon restart.
CREATE INDEX IF NOT EXISTS events_session_seq_idx ON events (session_id, seq);

-- Former migration: 0002_agent_session_id.sql

-- Resume support (design §7).
--
-- Record the downstream agent's OWN session id (the id claude-agent-acp /
-- codex-acp assigns at `session/new`). When a cowboy session's agent process
-- is gone — after a daemon restart, or an agent crash — a revived agent can
-- re-attach the prior conversation via ACP `session/load(agent_session_id)`
-- instead of opening a blank `session/new`. That is exactly how Zed keeps a
-- thread resumable forever.
--
-- NULL until the agent's first `session/new` returns an id, and stays NULL for
-- providers that don't support resume (the agent simply starts fresh on
-- revive). Adding it nullable means no backfill: pre-existing rows resume as
-- fresh sessions, same as before this migration.
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS agent_session_id text;

-- Former migration: 0003_pending.sql

-- Server-authoritative queue + drafts (cross-terminal sync).
--
-- The send-queue (prompts waiting for the current turn to finish) and parked
-- drafts used to live in each browser's localStorage, so they never synced
-- across devices/terminals. They are now owned by the daemon and broadcast to
-- every client. Persist both per session so staged messages survive a daemon
-- restart — matching the durability the old localStorage gave them.
--
-- Each is a JSONB array of { id, text, content } (content = the ACP content
-- blocks, empty for a plain-text message). Default '[]' means no backfill: every
-- existing session simply starts with empty lists.
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS queue jsonb NOT NULL DEFAULT '[]'::jsonb;
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS drafts jsonb NOT NULL DEFAULT '[]'::jsonb;

-- Former migration: 0004_session_position.sql

-- Manual session ordering (drag-to-arrange).
--
-- The session list was ordered by created_at. To let the user drag sessions
-- into a custom order (synced across terminals like queue/drafts), record an
-- explicit position per session. NULL until first reordered; load_all sorts by
-- `position ASC NULLS LAST, created_at ASC`, so never-reordered rows keep their
-- creation order and a partial reorder degrades gracefully. No backfill needed.
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS position double precision;

-- Former migration: 0005_session_soft_delete.sql

-- Soft-delete: a user "delete" now marks `deleted_at` instead of dropping the
-- row, and a background sweeper hard-deletes (cascade → events) rows whose
-- deleted_at is older than the retention window (3 days). This bounds the
-- storage a deleted session holds while leaving a recovery window, and `load_all`
-- skips soft-deleted rows so they vanish from the UI immediately.
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS deleted_at timestamptz;

-- Partial index so the sweeper's `WHERE deleted_at < ...` scan stays cheap.
CREATE INDEX IF NOT EXISTS sessions_deleted_at_idx ON sessions (deleted_at)
  WHERE deleted_at IS NOT NULL;

-- Former migration: 0006_auto_resume.sql

-- Auto-resume interrupted turns (design: tasks/active/session-auto-resume).
--
-- `sessions.auto_resume`: per-session OVERRIDE of the global default.
--   NULL  = inherit the global default (settings key 'session.autoResume.default')
--   true  = always auto-continue an interrupted turn for this session
--   false = never (explicit opt-out, e.g. when the global default is on)
-- Effective = COALESCE(auto_resume, global_default). Existing rows default to
-- NULL = inherit, so behavior is unchanged until a default/override is set.
--
-- `settings`: a small global key-value store (its first users are the
--   auto-resume default flag + the continuation-message template). `value` is
--   JSONB so new settings never need another migration.

ALTER TABLE sessions ADD COLUMN IF NOT EXISTS auto_resume boolean;

CREATE TABLE IF NOT EXISTS settings (
  key        text PRIMARY KEY,
  value      jsonb NOT NULL,
  updated_at timestamptz NOT NULL DEFAULT now()
);

-- Former migration: 0007_inference.sql

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

-- Former migration: 0008_turn_verdict.sql

-- Persist the confirm-detect turn-end verdict (awaiting_user / done) so a daemon
-- restart doesn't wipe a finished session's state. Previously transient — warm
-- restore hardcoded both to false on the theory "the next turn re-judges" — but a
-- DONE session has no next turn, so its green "Task complete" vanished on every
-- restart. Now durable: the judge's verdict is written here and restored into
-- SessionMeta on startup. Existing rows default to false (cleared), unchanged.
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS awaiting_user boolean NOT NULL DEFAULT false;
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS done boolean NOT NULL DEFAULT false;

-- Former migration: 0009_judge_runs.sql

-- Persist the confirm-detect judge-run HISTORY per session — backs the inspector
-- widget (long-press the turn-status pill → a list of recent judge runs, each
-- with its raw LLM input/output, deletable). Before this, a judge result was
-- broadcast live and only the LATEST verdict survived a restart (migration 0008);
-- the raw I/O was transient (the overlay's quick-peek expand). The full history
-- is now durable, server-authoritative, and cross-terminal — the daemon caps it
-- per session and owns add/delete/clear. A JSONB array (newest first) of
-- { id, at, layer, awaiting_user, done, confidence, reason, model, input, output,
-- cache_hit, cache_miss, latency_ms }. Default '[]' = no backfill; existing
-- sessions simply start with an empty history.
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS judge_runs jsonb NOT NULL DEFAULT '[]'::jsonb;

-- Former migration: 0010_system_session.sql

-- A "system" session is machine-driven and view-only: visible/watchable in the
-- UI, but the composer is hidden and user turns are rejected — only the backend
-- wake endpoint drives it. Used by the mnemosyne memory janitor. Persisted so a
-- system session survives a daemon restart. Mirrors 0008 (awaiting_user/done).
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS system boolean NOT NULL DEFAULT false;

-- Former migration: 0011_scheduled_wakeups.sql

-- One pending ScheduleWakeup per session, so an agent-armed wakeup survives a
-- daemon restart and still fires (the in-memory scheduler is re-armed from this
-- on startup). ON DELETE CASCADE auto-clears a deleted session's wakeup.
CREATE TABLE IF NOT EXISTS scheduled_wakeups (
    session_id  TEXT   PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
    fire_at_ms  BIGINT NOT NULL,
    prompt      TEXT   NOT NULL
);

-- Former migration: 0012_compact_event_log.sql

-- The primary key already provides the exact (session_id, seq) btree used by
-- every history query. The duplicate index costs the same space and doubles
-- write amplification.
DROP INDEX IF EXISTS events_session_seq_idx;

-- Runtime telemetry advances sessions.next_seq but has no transcript meaning.
DELETE FROM events
WHERE payload->>'kind' = 'update'
  AND payload->'update'->>'sessionUpdate' IN ('usage_update', 'session_info_update');

-- Older cowboy versions stored every tool progress frame as a full row. Fold
-- the last frame into the stable initial tool_call row, then drop the deltas.
-- toolCallId is session-local and stable for a call.
CREATE TEMP TABLE cowboy_tool_compaction AS
SELECT DISTINCT ON (initial.session_id, initial.payload->'update'->>'toolCallId')
  initial.session_id,
  initial.seq AS initial_seq,
  initial.payload AS initial_payload,
  delta.payload AS final_payload
FROM events AS initial
JOIN events AS delta
  ON delta.session_id = initial.session_id
 AND delta.payload->>'kind' = 'update'
 AND delta.payload->'update'->>'sessionUpdate' = 'tool_call_update'
 AND delta.payload->'update'->>'toolCallId' = initial.payload->'update'->>'toolCallId'
WHERE initial.payload->>'kind' = 'update'
  AND initial.payload->'update'->>'sessionUpdate' = 'tool_call'
ORDER BY
  initial.session_id,
  initial.payload->'update'->>'toolCallId',
  delta.seq DESC;

UPDATE events AS event
SET payload = jsonb_set(
  compact.initial_payload,
  '{update}',
  (compact.initial_payload->'update')
    || (compact.final_payload->'update')
    || '{"sessionUpdate":"tool_call"}'::jsonb
)
FROM cowboy_tool_compaction AS compact
WHERE event.session_id = compact.session_id
  AND event.seq = compact.initial_seq;

DELETE FROM events AS event
USING cowboy_tool_compaction AS compact
WHERE event.session_id = compact.session_id
  AND event.payload->>'kind' = 'update'
  AND event.payload->'update'->>'sessionUpdate' = 'tool_call_update'
  AND event.payload->'update'->>'toolCallId' = compact.initial_payload->'update'->>'toolCallId';

DROP TABLE cowboy_tool_compaction;

-- Former migration: 0013_drop_inference.sql

-- Cowboy now uses the host Codex app-server for confirm classification. Remove
-- the obsolete external-provider configuration and its stored API secrets.
DROP TABLE IF EXISTS inference_secrets;
DROP TABLE IF EXISTS inference_config;

-- Former migration: 0014_provider_actions.sql

-- One provider-level scheduled action. Reset credits are account-scoped, so a
-- session-owned timer could race another session and consume more than intended.
CREATE TABLE IF NOT EXISTS scheduled_provider_actions (
    provider        TEXT PRIMARY KEY,
    action           TEXT NOT NULL,
    fire_at_ms       BIGINT NOT NULL,
    idempotency_key  TEXT NOT NULL,
    CHECK (provider = 'codex'),
    CHECK (action = 'rate_limit_reset')
);

-- Former migration: 0015_provider_action_logs.sql

ALTER TABLE scheduled_provider_actions
    ADD COLUMN IF NOT EXISTS attempt_count INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS next_attempt_at_ms BIGINT;

UPDATE scheduled_provider_actions
SET next_attempt_at_ms = fire_at_ms
WHERE next_attempt_at_ms IS NULL;

CREATE TABLE IF NOT EXISTS provider_action_logs (
    id                  BIGSERIAL PRIMARY KEY,
    provider            TEXT NOT NULL,
    action              TEXT NOT NULL,
    trigger             TEXT NOT NULL,
    status              TEXT NOT NULL,
    phase               TEXT NOT NULL,
    message             TEXT NOT NULL,
    credit_id           TEXT,
    idempotency_suffix  TEXT,
    created_at_ms       BIGINT NOT NULL,
    CHECK (provider = 'codex'),
    CHECK (action = 'rate_limit_reset'),
    CHECK (trigger IN ('manual', 'scheduled')),
    CHECK (status IN ('scheduled', 'started', 'retrying', 'succeeded', 'failed', 'unknown', 'cancelled'))
);

CREATE INDEX IF NOT EXISTS provider_action_logs_created_idx
    ON provider_action_logs (created_at_ms DESC);

-- Former migration: 0016_mobile_review_state.sql

-- Mobile-only code-review workspace state.
--
-- This belongs to the Cowboy session rather than a browser installation:
-- iPhone/iPad clients share open source tabs, the active source, review mode,
-- and reviewed revisions. Desktop deliberately does not consume this state.

ALTER TABLE sessions
  ADD COLUMN IF NOT EXISTS mobile_review_state jsonb NOT NULL DEFAULT
    '{"mode":"git","tabs":[],"progress":{}}'::jsonb;

-- Former migration: 0017_machines.sql

-- Stable machine identities and immutable session placement.
--
-- `local` preserves every pre-multi-machine session and client. Remote
-- machines are enrolled separately; deleting or disconnecting one must not
-- rewrite the machine that originally owned a session.

CREATE TABLE IF NOT EXISTS machines (
    id text PRIMARY KEY,
    display_name text NOT NULL,
    connection_mode text NOT NULL,
    platform text NOT NULL,
    architecture text NOT NULL,
    status text NOT NULL DEFAULT 'offline',
    protocol_min integer NOT NULL DEFAULT 1,
    protocol_max integer NOT NULL DEFAULT 1,
    inventory jsonb NOT NULL DEFAULT '[]'::jsonb,
    last_seen_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK (connection_mode IN ('local', 'outbound_wss')),
    CHECK (status IN ('online', 'offline', 'updating', 'degraded'))
);

INSERT INTO machines (
    id, display_name, connection_mode, platform, architecture, status,
    last_seen_at
) VALUES (
    'local', 'This machine', 'local', 'unknown', 'unknown', 'online', now()
) ON CONFLICT (id) DO NOTHING;

ALTER TABLE sessions
    ADD COLUMN IF NOT EXISTS machine_id text NOT NULL DEFAULT 'local';

ALTER TABLE sessions
    ADD CONSTRAINT sessions_machine_id_fkey
    FOREIGN KEY (machine_id) REFERENCES machines(id) ON DELETE RESTRICT;

CREATE INDEX IF NOT EXISTS sessions_machine_id_idx ON sessions(machine_id);

-- Former migration: 0018_machine_enrollment.sql

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

-- Former migration: 0019_unify_local_machine.sql

-- Hawk is a normal authenticated Machine. Preserve immutable placement while
-- replacing the controller-created legacy identity with its stable host id.

INSERT INTO machines (
    id, display_name, connection_mode, platform, architecture, status
) VALUES (
    'hawk', 'Hawk', 'local', 'linux', 'x86_64', 'offline'
) ON CONFLICT (id) DO NOTHING;

UPDATE sessions SET machine_id = 'hawk' WHERE machine_id = 'local';
DELETE FROM machines WHERE id = 'local';

-- Former migration: 0020_machine_reconnect_grace.sql

-- Preserve Machine presence across a short Cowboy controller restart.

ALTER TABLE machines
    DROP CONSTRAINT IF EXISTS machines_status_check;

ALTER TABLE machines
    ADD CONSTRAINT machines_status_check
    CHECK (status IN ('online', 'reconnecting', 'offline', 'updating', 'degraded'));

ALTER TABLE machines
    ADD COLUMN IF NOT EXISTS reconnect_deadline_at timestamptz;

-- Former migration: 0021_runtime_incidents.sql

-- Runtime Incident Ledger. Raw evidence stays in VictoriaLogs/VictoriaMetrics;
-- PostgreSQL keeps the durable, searchable incident identity and recovery
-- outcome so transcript events are not overloaded with observability data.
CREATE TABLE IF NOT EXISTS runtime_incidents (
  id                  text PRIMARY KEY,
  occurred_at         timestamptz NOT NULL,
  received_at         timestamptz NOT NULL DEFAULT now(),
  updated_at          timestamptz NOT NULL DEFAULT now(),
  source              text NOT NULL,
  classification      text NOT NULL,
  severity            text NOT NULL,
  state               text NOT NULL,
  summary             text NOT NULL,
  fingerprint         text NOT NULL,
  session_id          text,
  client_id           text,
  machine_id          text,
  trace_id            text,
  build               text,
  evidence_start      timestamptz NOT NULL,
  evidence_end        timestamptz NOT NULL,
  detail              jsonb NOT NULL DEFAULT '{}'::jsonb,
  recovered_at        timestamptz,
  recovery_outcome    text
);

CREATE INDEX IF NOT EXISTS runtime_incidents_occurred_idx
  ON runtime_incidents (occurred_at DESC);
CREATE INDEX IF NOT EXISTS runtime_incidents_session_idx
  ON runtime_incidents (session_id, occurred_at DESC)
  WHERE session_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS runtime_incidents_fingerprint_idx
  ON runtime_incidents (fingerprint, occurred_at DESC);
CREATE INDEX IF NOT EXISTS runtime_incidents_active_idx
  ON runtime_incidents (occurred_at DESC)
  WHERE state <> 'recovered';

-- Former migration: 0022_provider_usage_ledger.sql

-- Cowboy-owned provider usage ledger. Provider account facts remain in their
-- adapters; these rows describe only requests observed by Cowboy gateways.
CREATE TABLE provider_usage_events (
    machine_id text NOT NULL REFERENCES machines(id) ON DELETE RESTRICT,
    producer_id text NOT NULL,
    sequence bigint NOT NULL,
    occurred_at timestamptz NOT NULL,
    received_at timestamptz NOT NULL DEFAULT now(),
    account_fingerprint text NOT NULL,
    provider text NOT NULL,
    agent text NOT NULL,
    model text,
    status integer NOT NULL,
    input_tokens bigint,
    output_tokens bigint,
    reasoning_tokens bigint,
    cache_hit_tokens bigint,
    cache_miss_tokens bigint,
    schema_version integer NOT NULL DEFAULT 1,
    PRIMARY KEY (machine_id, producer_id, sequence),
    CHECK (provider IN ('deepseek')),
    CHECK (agent IN ('codex', 'claude')),
    CHECK (status BETWEEN 100 AND 599),
    CHECK (input_tokens IS NULL OR input_tokens >= 0),
    CHECK (output_tokens IS NULL OR output_tokens >= 0),
    CHECK (reasoning_tokens IS NULL OR reasoning_tokens >= 0),
    CHECK (cache_hit_tokens IS NULL OR cache_hit_tokens >= 0),
    CHECK (cache_miss_tokens IS NULL OR cache_miss_tokens >= 0)
);

CREATE INDEX provider_usage_provider_time_idx
    ON provider_usage_events (provider, account_fingerprint, received_at DESC);
CREATE INDEX provider_usage_agent_time_idx
    ON provider_usage_events (provider, agent, received_at DESC);
CREATE INDEX provider_usage_machine_time_idx
    ON provider_usage_events (machine_id, received_at DESC);

CREATE TABLE provider_usage_producers (
    machine_id text NOT NULL REFERENCES machines(id) ON DELETE RESTRICT,
    producer_id text NOT NULL,
    provider text NOT NULL,
    account_fingerprint text NOT NULL,
    agent text NOT NULL,
    last_sequence bigint NOT NULL,
    last_occurred_at timestamptz NOT NULL,
    last_received_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (machine_id, producer_id)
);

-- Former migration: 0023_session_workspace_identity.sql

-- Preserve the selected Machine workspace independently from the isolated
-- per-session cwd. Older rows remain unknown rather than guessing identity
-- from an editable title or a transient worktree path.
ALTER TABLE sessions
    ADD COLUMN workspace_id text,
    ADD COLUMN workspace_name text,
    ADD COLUMN workspace_source_path text;

-- Former migration: 0024_provider_usage_telemetry_v2.sql

-- Content-free request-shape telemetry for cache and reliability analysis.
-- Existing rows remain explicitly legacy and are excluded from verified cache
-- rates because version one could not distinguish absent from inferred data.
ALTER TABLE provider_usage_events
    ADD COLUMN operation text NOT NULL DEFAULT 'legacy',
    ADD COLUMN protocol text NOT NULL DEFAULT 'legacy',
    ADD COLUMN cache_observation text NOT NULL DEFAULT 'legacy',
    ADD COLUMN usage_observed boolean,
    ADD COLUMN completed boolean,
    ADD COLUMN streaming boolean,
    ADD COLUMN duration_ms bigint,
    ADD COLUMN request_bytes bigint,
    ADD COLUMN input_item_count bigint,
    ADD COLUMN tool_count bigint,
    ADD COLUMN system_block_count bigint,
    ADD COLUMN has_previous_response_id boolean,
    ADD COLUMN compatibility_fixes bigint,
    ADD CONSTRAINT provider_usage_operation_check
        CHECK (operation IN ('legacy', 'responses', 'compact', 'messages')),
    ADD CONSTRAINT provider_usage_protocol_check
        CHECK (protocol IN ('legacy', 'responses', 'chat_completions', 'anthropic_messages')),
    ADD CONSTRAINT provider_usage_cache_observation_check
        CHECK (cache_observation IN ('legacy', 'absent', 'derived', 'explicit')),
    ADD CONSTRAINT provider_usage_duration_check
        CHECK (duration_ms IS NULL OR duration_ms >= 0),
    ADD CONSTRAINT provider_usage_request_bytes_check
        CHECK (request_bytes IS NULL OR request_bytes >= 0),
    ADD CONSTRAINT provider_usage_input_item_count_check
        CHECK (input_item_count IS NULL OR input_item_count >= 0),
    ADD CONSTRAINT provider_usage_tool_count_check
        CHECK (tool_count IS NULL OR tool_count >= 0),
    ADD CONSTRAINT provider_usage_system_block_count_check
        CHECK (system_block_count IS NULL OR system_block_count >= 0),
    ADD CONSTRAINT provider_usage_compatibility_fixes_check
        CHECK (compatibility_fixes IS NULL OR compatibility_fixes BETWEEN 0 AND 1000000),
    ADD CONSTRAINT provider_usage_token_upper_bound_check
        CHECK (
            (input_tokens IS NULL OR input_tokens BETWEEN 0 AND 10000000) AND
            (output_tokens IS NULL OR output_tokens BETWEEN 0 AND 10000000) AND
            (reasoning_tokens IS NULL OR reasoning_tokens BETWEEN 0 AND 10000000) AND
            (cache_hit_tokens IS NULL OR cache_hit_tokens BETWEEN 0 AND 10000000) AND
            (cache_miss_tokens IS NULL OR cache_miss_tokens BETWEEN 0 AND 10000000)
        ),
    ADD CONSTRAINT provider_usage_duration_upper_bound_check
        CHECK (duration_ms IS NULL OR duration_ms <= 86400000),
    ADD CONSTRAINT provider_usage_request_bytes_upper_bound_check
        CHECK (request_bytes IS NULL OR request_bytes <= 67108864),
    ADD CONSTRAINT provider_usage_shape_upper_bound_check
        CHECK (
            (input_item_count IS NULL OR input_item_count <= 1000000) AND
            (tool_count IS NULL OR tool_count <= 1000000) AND
            (system_block_count IS NULL OR system_block_count <= 1000000)
        );

CREATE INDEX provider_usage_provider_occurred_idx
    ON provider_usage_events (provider, occurred_at DESC);

-- Former migration: 0025_provider_usage_telemetry_v3.sql

-- Provider-neutral request lineage and prefix telemetry. Fingerprints are
-- runtime-namespaced, credential-keyed HMACs produced by each isolated gateway; no prompt,
-- response, session id, tool payload, or credential is stored.
ALTER TABLE provider_usage_events
    DROP CONSTRAINT provider_usage_events_agent_check,
    DROP CONSTRAINT provider_usage_operation_check,
    ADD COLUMN model_family text NOT NULL DEFAULT 'unknown',
    ADD COLUMN resolved_model text,
    ADD COLUMN model_revision text,
    ADD COLUMN request_role text NOT NULL DEFAULT 'unknown',
    ADD COLUMN client_protocol text NOT NULL DEFAULT 'legacy',
    ADD COLUMN upstream_protocol text NOT NULL DEFAULT 'legacy',
    ADD COLUMN translation_mode text NOT NULL DEFAULT 'legacy',
    ADD COLUMN thinking_mode text NOT NULL DEFAULT 'unknown',
    ADD COLUMN reasoning_effort text NOT NULL DEFAULT 'unknown',
    ADD COLUMN session_fingerprint text,
    ADD COLUMN session_attribution text NOT NULL DEFAULT 'unattributed',
    ADD COLUMN traffic_source text NOT NULL DEFAULT 'unattributed',
    ADD COLUMN static_prefix_fingerprint text,
    ADD COLUMN request_prefix_fingerprint text,
    ADD COLUMN gateway_build text,
    ADD COLUMN gateway_boot_id text,
    ADD CONSTRAINT provider_usage_agent_slug_check
        CHECK (agent ~ '^[a-z][a-z0-9_-]{0,31}$'),
    ADD CONSTRAINT provider_usage_operation_v3_check
        CHECK (operation IN ('legacy', 'responses', 'compact', 'messages', 'chat_completions')),
    ADD CONSTRAINT provider_usage_model_family_check
        CHECK (model_family IN ('unknown', 'flash', 'pro')),
    ADD CONSTRAINT provider_usage_request_role_check
        CHECK (request_role IN ('unknown', 'executor', 'planner', 'subagent', 'reviewer')),
    ADD CONSTRAINT provider_usage_client_protocol_check
        CHECK (client_protocol IN ('legacy', 'responses', 'chat_completions', 'anthropic_messages')),
    ADD CONSTRAINT provider_usage_upstream_protocol_check
        CHECK (upstream_protocol IN ('legacy', 'responses', 'chat_completions', 'anthropic_messages')),
    ADD CONSTRAINT provider_usage_translation_mode_check
        CHECK (translation_mode IN ('legacy', 'native', 'responses_to_chat', 'anthropic_compat')),
    ADD CONSTRAINT provider_usage_thinking_mode_check
        CHECK (thinking_mode IN ('unknown', 'enabled', 'disabled')),
    ADD CONSTRAINT provider_usage_reasoning_effort_check
        CHECK (reasoning_effort IN ('unknown', 'default', 'low', 'high', 'max')),
    ADD CONSTRAINT provider_usage_session_attribution_check
        CHECK (session_attribution IN ('unattributed', 'response_lineage', 'prefix_root', 'explicit')),
    ADD CONSTRAINT provider_usage_traffic_source_check
        CHECK (traffic_source IN ('unattributed', 'cowboy')),
    ADD CONSTRAINT provider_usage_session_fingerprint_check
        CHECK (session_fingerprint IS NULL OR session_fingerprint ~ '^[0-9a-f]{32}$'),
    ADD CONSTRAINT provider_usage_static_prefix_fingerprint_check
        CHECK (static_prefix_fingerprint IS NULL OR static_prefix_fingerprint ~ '^[0-9a-f]{32}$'),
    ADD CONSTRAINT provider_usage_request_prefix_fingerprint_check
        CHECK (request_prefix_fingerprint IS NULL OR request_prefix_fingerprint ~ '^[0-9a-f]{32}$'),
    ADD CONSTRAINT provider_usage_gateway_build_check
        CHECK (gateway_build IS NULL OR gateway_build ~ '^[0-9a-f]{16}$'),
    ADD CONSTRAINT provider_usage_gateway_boot_id_check
        CHECK (gateway_boot_id IS NULL OR gateway_boot_id ~ '^[0-9a-f]{16}$');

CREATE INDEX provider_usage_model_family_time_idx
    ON provider_usage_events (provider, model_family, occurred_at DESC);
CREATE INDEX provider_usage_role_time_idx
    ON provider_usage_events (provider, agent, request_role, occurred_at DESC);
CREATE INDEX provider_usage_session_time_idx
    ON provider_usage_events (provider, account_fingerprint, agent, session_fingerprint, occurred_at DESC)
    WHERE session_fingerprint IS NOT NULL;

-- Former migration: 0026_session_config.sql

-- Persist the agent-advertised config surface and the user's session-owned
-- choices. `config_options` is the latest display snapshot; `config_preferences`
-- contains only values selected by the user (or seeded defaults), so a provider
-- can refresh its option list without erasing the session's intent.
ALTER TABLE sessions
    ADD COLUMN config_options jsonb,
    ADD COLUMN config_preferences jsonb NOT NULL DEFAULT '{}'::jsonb;

-- Before this migration Cowboy had no durable per-session config choices, so an
-- empty preference object means the session was using the provider default. Make
-- the new OpenAI default apply to those existing sessions as well; subsequent
-- user changes replace these values through the service write path.
UPDATE sessions
SET config_preferences = '{"model":"gpt-5.6-luna","reasoning_effort":"max"}'::jsonb
WHERE provider = 'codex' AND config_preferences = '{}'::jsonb;

-- Former migration: 0027_deepseek_cache_keepalive.sql

-- Distinguish background DeepSeek cache keepalives from interactive agent
-- requests. Keepalive rows retain only content-free request lineage and timing;
-- request snapshots stay process-local in the gateway and never enter Postgres.
ALTER TABLE provider_usage_events
    ADD COLUMN request_purpose text NOT NULL DEFAULT 'interactive',
    ADD COLUMN cache_keepalive_outcome text NOT NULL DEFAULT 'not_applicable',
    ADD COLUMN cache_keepalive_algorithm text,
    ADD COLUMN cache_keepalive_attempt bigint,
    ADD COLUMN cache_keepalive_interval_ms bigint,
    ADD COLUMN cache_keepalive_source_age_ms bigint,
    ADD COLUMN source_request_prefix_fingerprint text,
    ADD CONSTRAINT provider_usage_request_purpose_check
        CHECK (request_purpose IN ('interactive', 'cache_keepalive')),
    ADD CONSTRAINT provider_usage_cache_keepalive_outcome_check
        CHECK (cache_keepalive_outcome IN (
            'not_applicable', 'hit', 'miss', 'partial', 'retryable_error',
            'terminal_error', 'preempted'
        )),
    ADD CONSTRAINT provider_usage_cache_keepalive_algorithm_check
        CHECK (
            cache_keepalive_algorithm IS NULL OR
            cache_keepalive_algorithm ~ '^[a-z][a-z0-9_-]{0,63}$'
        ),
    ADD CONSTRAINT provider_usage_cache_keepalive_attempt_check
        CHECK (cache_keepalive_attempt IS NULL OR cache_keepalive_attempt BETWEEN 0 AND 1000),
    ADD CONSTRAINT provider_usage_cache_keepalive_interval_check
        CHECK (
            cache_keepalive_interval_ms IS NULL OR
            cache_keepalive_interval_ms BETWEEN 0 AND 604800000
        ),
    ADD CONSTRAINT provider_usage_cache_keepalive_source_age_check
        CHECK (
            cache_keepalive_source_age_ms IS NULL OR
            cache_keepalive_source_age_ms BETWEEN 0 AND 604800000
        ),
    ADD CONSTRAINT provider_usage_source_request_prefix_check
        CHECK (
            source_request_prefix_fingerprint IS NULL OR
            source_request_prefix_fingerprint ~ '^[0-9a-f]{32}$'
        ),
    ADD CONSTRAINT provider_usage_cache_keepalive_shape_check
        CHECK (
            (
                request_purpose = 'interactive' AND
                cache_keepalive_outcome = 'not_applicable' AND
                cache_keepalive_algorithm IS NULL AND
                coalesce(cache_keepalive_attempt, 0) = 0 AND
                coalesce(cache_keepalive_interval_ms, 0) = 0 AND
                coalesce(cache_keepalive_source_age_ms, 0) = 0 AND
                source_request_prefix_fingerprint IS NULL
            ) OR (
                request_purpose = 'cache_keepalive' AND
                session_attribution = 'explicit' AND
                traffic_source = 'cowboy' AND
                cache_keepalive_outcome <> 'not_applicable' AND
                cache_keepalive_algorithm IS NOT NULL AND
                cache_keepalive_attempt >= 1 AND
                cache_keepalive_interval_ms > 0 AND
                cache_keepalive_source_age_ms >= 0 AND
                source_request_prefix_fingerprint IS NOT NULL
            )
        );

CREATE INDEX provider_usage_purpose_time_idx
    ON provider_usage_events (provider, request_purpose, occurred_at DESC);
CREATE INDEX provider_usage_keepalive_outcome_time_idx
    ON provider_usage_events (provider, cache_keepalive_outcome, occurred_at DESC)
    WHERE request_purpose = 'cache_keepalive';

UPDATE sessions
SET config_preferences = config_preferences || '{"deepseek_cache_protection":true}'::jsonb
WHERE provider IN ('claude-deepseek', 'codex-deepseek')
  AND NOT config_preferences ? 'deepseek_cache_protection';

-- Former migration: 0028_xai_provider_actions.sql

ALTER TABLE scheduled_provider_actions
    DROP CONSTRAINT IF EXISTS scheduled_provider_actions_provider_check;

ALTER TABLE scheduled_provider_actions
    ADD CONSTRAINT scheduled_provider_actions_provider_check
    CHECK (provider IN ('codex', 'xai'));

ALTER TABLE provider_action_logs
    DROP CONSTRAINT IF EXISTS provider_action_logs_provider_check;

ALTER TABLE provider_action_logs
    ADD CONSTRAINT provider_action_logs_provider_check
    CHECK (provider IN ('codex', 'xai'));

-- Former migration: 0029_provider_platform.sql

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

-- Former migration: 0030_crash_incident_severity.sql

-- Crash incidents predate the critical severity policy. Reclassify only
-- unambiguous runtime, startup, and application crashes; ordinary command
-- failures and intentional interruptions retain their original severity.
UPDATE runtime_incidents
SET severity = 'critical',
    classification = CASE
      WHEN summary ILIKE '%did not become ready%'
        OR summary ILIKE '%exited before readiness%'
        OR summary ILIKE '%generation launch failed%'
      THEN 'worker_startup_failure'
      ELSE classification
    END,
    updated_at = now()
WHERE severity <> 'critical'
  AND (
    classification IN (
      'runtime_failure',
      'process_exit',
      'resource_exhaustion',
      'worker_startup_failure',
      'client_render_failure',
      'client_window_error',
      'client_unhandled_rejection'
    )
    OR (
      id LIKE 'lifecycle:%'
      AND classification IN ('protocol_failure', 'transport_failure')
    )
    OR summary ILIKE '%did not become ready%'
    OR summary ILIKE '%exited before readiness%'
    OR summary ILIKE '%generation launch failed%'
  );

-- Former migration: 0031_product_users.sql

-- Product users, login sessions, and unused API-token storage.
-- sessions.owner_user_id is shipped nullable and unread until the stamp PR.

CREATE TABLE users (
    id            text PRIMARY KEY,
    username      text NOT NULL UNIQUE
                  CHECK (
                      username = lower(btrim(username))
                      AND username ~ '^[a-z0-9._-]{1,64}$'
                  ),
    password_algo text NOT NULL,
    password_hash text NOT NULL,
    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now(),
    disabled_at   timestamptz
);

CREATE TABLE user_sessions (
    token_hash    text PRIMARY KEY,
    user_id       text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at    timestamptz NOT NULL DEFAULT now(),
    expires_at    timestamptz NOT NULL,
    last_seen_at  timestamptz NOT NULL DEFAULT now(),
    user_agent    text
);

CREATE INDEX user_sessions_user_id_idx ON user_sessions(user_id);
CREATE INDEX user_sessions_expires_idx ON user_sessions(expires_at);

CREATE TABLE user_api_tokens (
    id            text PRIMARY KEY,
    user_id       text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name          text NOT NULL,
    token_prefix  text NOT NULL,
    token_hash    text NOT NULL UNIQUE,
    created_at    timestamptz NOT NULL DEFAULT now(),
    expires_at    timestamptz,
    last_used_at  timestamptz,
    revoked_at    timestamptz
);

ALTER TABLE sessions
    ADD COLUMN owner_user_id text REFERENCES users(id) ON DELETE SET NULL;

CREATE INDEX sessions_owner_user_id_idx ON sessions(owner_user_id);

-- Former migration: 0032_user_passkeys.sql

-- Discoverable WebAuthn credentials and the product viewing lock clock.

ALTER TABLE users
    ADD COLUMN passkey_reauth_enabled boolean NOT NULL DEFAULT true;

ALTER TABLE users
    ADD COLUMN last_step_up_at timestamptz;

UPDATE users
SET last_step_up_at = COALESCE(last_step_up_at, updated_at, now());

CREATE TABLE user_passkeys (
    id            text PRIMARY KEY,
    user_id       text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    credential_id text NOT NULL UNIQUE,
    nickname      text NOT NULL
                  CHECK (char_length(nickname) BETWEEN 1 AND 64),
    passkey_json  text NOT NULL,
    created_at    timestamptz NOT NULL DEFAULT now(),
    last_used_at  timestamptz
);

CREATE INDEX user_passkeys_user_id_idx ON user_passkeys(user_id);

-- Former migration: 0033_admin_passkeys.sql

-- Admin-plane WebAuthn credentials. Distinct from product user_passkeys.

CREATE TABLE admin_passkeys (
    id            text PRIMARY KEY,
    account       text NOT NULL,
    credential_id text NOT NULL UNIQUE,
    nickname      text NOT NULL
                  CHECK (char_length(nickname) BETWEEN 1 AND 64),
    passkey_json  text NOT NULL,
    created_at    timestamptz NOT NULL DEFAULT now(),
    last_used_at  timestamptz
);

CREATE INDEX admin_passkeys_account_idx ON admin_passkeys(account);


-- Preserve predecessor rollback compatibility for databases first created
-- from this consolidated baseline. SQLx validates only version and checksum.
INSERT INTO _sqlx_migrations
    (version, description, success, checksum, execution_time)
VALUES
    (1, 'legacy compatibility', TRUE, decode('6b8c96bfaa20be52c2cc27d985c0c5409533b375a4a9443d8fb9aef0f86fb7b5cacb97b2f504006cd23c76cdf820193f', 'hex'), 0),
    (2, 'legacy compatibility', TRUE, decode('ad82ac0a149e30e4510cea7841ca731a80fb1cda357a65a5a2649edd46d7bf703d93fd74af333c0356a48a0e3540b453', 'hex'), 0),
    (3, 'legacy compatibility', TRUE, decode('2d72276733684ccf051498bf4d837dfa0227ff58fe60d6a65814e7417b269090cb84194821025c3b8ca1822932c50c3e', 'hex'), 0),
    (4, 'legacy compatibility', TRUE, decode('9719a53de1f71840487fc264672aa54f9e3738c94a5b5f370795d4a0941621e1c818b2e393e089371622e7555c1d182c', 'hex'), 0),
    (5, 'legacy compatibility', TRUE, decode('32f2518e8b36acc3657511cefae4eb3e188f0b5af162eb649e2de5684e1d2be141df4e3974966811ab168256e97f1c17', 'hex'), 0),
    (6, 'legacy compatibility', TRUE, decode('c48955b47d0d98fe31a67c88f576bf0ff413e758882c694b833f7bc7d0421b8de6a143099177b4f73e56c6978f1bdc02', 'hex'), 0),
    (7, 'legacy compatibility', TRUE, decode('17a031e99e34cc7dc7cfc34cee4ce4ccf7513733168d013fa442a450a1774f571b111f9b312780588b72b7b32c84e2e1', 'hex'), 0),
    (8, 'legacy compatibility', TRUE, decode('cffedc29c0204aa8b850ec933028ad5ae956298ea75197e30e1f7f776372c1c022b0a2ade8c8e5ac06b2604dacce269c', 'hex'), 0),
    (9, 'legacy compatibility', TRUE, decode('8414e8b198e54343f829d7c30a3a54fd3b6b9248d49959657ae6ae7f495382e9375373e346128bbfe785bc35184f5900', 'hex'), 0),
    (10, 'legacy compatibility', TRUE, decode('0a06f9bf0b90ed61dc3b51895af4c583d1c3cdaec9587115ce4960e1c97c1cb976f995c3ef076852ddab0f932dcc6992', 'hex'), 0),
    (11, 'legacy compatibility', TRUE, decode('27fa174e8727c73555744859fd9a7d5ca2da032bc7cd8170414ad6fbeddc900f162663019dc2e917bff35f301f8245be', 'hex'), 0),
    (12, 'legacy compatibility', TRUE, decode('43d8a811f10bdfc95c8a04ea22700da71c4e040e7bc4f2e01eb1368db963d2d45056c2f16af3016ea1f644b0a0b0744b', 'hex'), 0),
    (13, 'legacy compatibility', TRUE, decode('bb0861722480a3420fa57d7c1cd3fc11b21b5d3d0454f2cd248b87cdde08c0af9c9a6da86fcc89132b478f85d63fa79e', 'hex'), 0),
    (14, 'legacy compatibility', TRUE, decode('4c052d863e01b316a0efd2b3a213ca13b0ddbc5b0a5e1d6958f57b9042c6edffea9cab18f8bd454717060cb6ebfd82a4', 'hex'), 0),
    (15, 'legacy compatibility', TRUE, decode('1dccce5e4bd651d8a038a9f1d0eed5cbc7b0a179569e1549a261936b9fff3c94a91b4ff86c933c886c7ac3ed1ca08c93', 'hex'), 0),
    (16, 'legacy compatibility', TRUE, decode('4a4e5fd713578d2e4e9aabad317c636b2960cf146463144c2ac3db3b2867ddcb8b90f13aa485ce7018f40cdc86bbf260', 'hex'), 0),
    (17, 'legacy compatibility', TRUE, decode('6c7058962c632b1d781053271ac4f2698d96a1679c6b25f3b64a54580525ff2f4de7fcd2a4d5570d907bb19d608ff590', 'hex'), 0),
    (18, 'legacy compatibility', TRUE, decode('4a7e4af23cb326e80f0fe3537d7ba869301a2c173711429ec848375a060eb374facd21971cd270fbe228c60ae3e1c2c1', 'hex'), 0),
    (19, 'legacy compatibility', TRUE, decode('ffa7ba7597d169f2ac2c7232393b765f598c144e011e6e6f32c4972ea809e21ea0373749fbccd327c1bc22e4abf28130', 'hex'), 0),
    (20, 'legacy compatibility', TRUE, decode('882faa5ac6d9b85380ced22d833874d901a00fe5b3e30e900728187196e9308d520923fb1bc8661c8b27b0ad51ad7245', 'hex'), 0),
    (21, 'legacy compatibility', TRUE, decode('8d1a9c226c7a47a4b8e0969459a8fc04b25fec29da04409947c15a6e42f31c4a12e02e8bdcec92eb3e7d01ae8298caf7', 'hex'), 0),
    (22, 'legacy compatibility', TRUE, decode('8ec1498e475d85235ac69485337156dc691cd983d74ff4d545143f2f5fe3bc64e237998c9c344a0fc15cd128faeea4fb', 'hex'), 0),
    (23, 'legacy compatibility', TRUE, decode('47c9c01db17a1ed2d555e4e158a396c52aa6995de62ff6d2e2395217514fba51e1688bee7c8e3825f0d5bcca496d8233', 'hex'), 0),
    (24, 'legacy compatibility', TRUE, decode('dcea269ba3b7febe962e372c65942d54d958cdae7cbe6e9fbbf335eb1c82267a6fcbc36cf6d13d1810ea896def82a663', 'hex'), 0),
    (25, 'legacy compatibility', TRUE, decode('3cc2b5faad457baa415ef117a2c57ff5298fe00d384e7e3ee777bd99405ea95d1e92b9959a758aff110d96556a521163', 'hex'), 0),
    (26, 'legacy compatibility', TRUE, decode('260511cce91f76be818ee39816c084710535501b01f6d8e38c5a2bb7f904c140d3ac807554bf052bb3b368f427e229e8', 'hex'), 0),
    (27, 'legacy compatibility', TRUE, decode('e5bc0cb900f35b0505d336bbe66855e4f25d9a7bd90c282cdfcb5c72d0666103a52fc82895aba1687ee3110d54949cdb', 'hex'), 0),
    (28, 'legacy compatibility', TRUE, decode('0000b3b61491566bbcfe844d61935ef11fab13841a535161d41e134aa8f98011e8085fc0311ceb6486dee7f34770dbbd', 'hex'), 0),
    (29, 'legacy compatibility', TRUE, decode('82c8276484865a2a62cb14088d7c3aea5a57fa992cf67ff35f89e753f1c9acdec14febe2f7303a4d4c12aff6f7db00c5', 'hex'), 0),
    (30, 'legacy compatibility', TRUE, decode('24e44a3eac807a8dfab114e852d2656c0e703d9e44b8c21ff6aa4557503b95df3d6a2b542b407678779ad75e604f391b', 'hex'), 0),
    (31, 'legacy compatibility', TRUE, decode('caaf278983d41a30d5673e519073a6a7131635396253f6f1157ad9613172b0bb9b4b01c88889c56ddcf057b1472e5bed', 'hex'), 0),
    (32, 'legacy compatibility', TRUE, decode('dbfe7732245c44427ba727d16231c3bc6bf8d3dcaee5fe2eca48d200d103131bf3ae6636fa3768acfa5b24078f15f8f8', 'hex'), 0),
    (33, 'legacy compatibility', TRUE, decode('cabf4ea80cf9891e1bff94e4a831b1b0a94ef7f37caa96b127c77c901cac4778c1de9fb1e471b6568121e1e42144b170', 'hex'), 0)
ON CONFLICT (version) DO NOTHING;
