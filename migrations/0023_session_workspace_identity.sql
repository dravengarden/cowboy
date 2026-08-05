-- Preserve the selected Machine workspace independently from the isolated
-- per-session cwd. Older rows remain unknown rather than guessing identity
-- from an editable title or a transient worktree path.
ALTER TABLE sessions
    ADD COLUMN workspace_id text,
    ADD COLUMN workspace_name text,
    ADD COLUMN workspace_source_path text;
