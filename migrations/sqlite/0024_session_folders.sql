-- Sessions-sidebar folders (docs/sessions-folders.md). Additive: a folder row
-- is owned by one product user (NULL while product auth is disabled), nests
-- through parent_id, and may bind one project label. sessions.folder_id holds
-- only explicit placements; no FK so a vanished folder degrades to the root.
CREATE TABLE session_folders (
    id TEXT PRIMARY KEY,
    owner_user_id TEXT,
    name TEXT NOT NULL,
    parent_id TEXT,
    project TEXT,
    position INTEGER NOT NULL DEFAULT 0,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
CREATE INDEX session_folders_owner_idx ON session_folders (owner_user_id);
ALTER TABLE sessions ADD COLUMN folder_id TEXT;
