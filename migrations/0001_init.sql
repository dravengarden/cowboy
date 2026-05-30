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
