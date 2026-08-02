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
