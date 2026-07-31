-- Hawk is a normal authenticated Machine. Preserve immutable placement while
-- replacing the controller-created legacy identity with its stable host id.

INSERT INTO machines (
    id, display_name, connection_mode, platform, architecture, status
) VALUES (
    'hawk', 'Hawk', 'local', 'linux', 'x86_64', 'offline'
) ON CONFLICT (id) DO NOTHING;

UPDATE sessions SET machine_id = 'hawk' WHERE machine_id = 'local';
DELETE FROM machines WHERE id = 'local';
