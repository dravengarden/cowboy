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
