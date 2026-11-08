-- Cowboy Widgets (docs/widgets.md §6.3): the authoritative widget document of
-- a session; the transcript row is a projection of it. `cleared` tombstones a
-- widget whose transcript was cleared so it is never published again. Older
-- Controllers ignore this table.
CREATE TABLE IF NOT EXISTS session_widgets (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    widget_id TEXT NOT NULL,
    handle TEXT NOT NULL,
    revision BIGINT NOT NULL,
    state_version BIGINT NOT NULL,
    closed BOOLEAN NOT NULL DEFAULT FALSE,
    cleared BOOLEAN NOT NULL DEFAULT FALSE,
    document TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL,
    PRIMARY KEY (session_id, widget_id)
);
CREATE UNIQUE INDEX IF NOT EXISTS session_widgets_handle ON session_widgets (session_id, handle);
