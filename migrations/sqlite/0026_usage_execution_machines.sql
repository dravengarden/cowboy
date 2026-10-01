CREATE TABLE usage_execution_machines (
    account TEXT PRIMARY KEY,
    machine_id TEXT NOT NULL REFERENCES machines(id)
);
