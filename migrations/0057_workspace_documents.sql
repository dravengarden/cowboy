-- One workspace navigation for independent documents and live sessions.
CREATE TABLE draft_workspace_folder_imports (
    owner_user_id TEXT NOT NULL,
    document_id TEXT NOT NULL,
    folder_id TEXT NOT NULL UNIQUE,
    PRIMARY KEY (owner_user_id, document_id)
);
CREATE TABLE workspace_item_order (
    owner_user_id TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
