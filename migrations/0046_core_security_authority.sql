-- Empty until the separately authorized stopped-Controller ownership handoff.
CREATE TABLE core_security_authority (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    authority TEXT NOT NULL,
    phase TEXT NOT NULL CHECK (phase IN ('prepared', 'ready')),
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL
);
