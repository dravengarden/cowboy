-- App-owned immutable plans and execution reservations, never persisted authority.
CREATE TABLE cardea_operation_capacity (singleton INTEGER PRIMARY KEY CHECK (singleton = 1), retained INTEGER NOT NULL CHECK (retained BETWEEN 0 AND 4096));
INSERT INTO cardea_operation_capacity VALUES (1, 0);
CREATE TABLE cardea_operations (
    operation_id TEXT PRIMARY KEY,
    request_key TEXT NOT NULL UNIQUE,
    request_digest TEXT NOT NULL,
    plan TEXT NOT NULL,
    claim_id TEXT,
    state TEXT NOT NULL CHECK (state IN ('prepared', 'executing', 'completed')),
    receipt TEXT
);
