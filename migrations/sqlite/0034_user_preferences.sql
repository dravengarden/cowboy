CREATE TABLE user_preference_owners (
    owner_user_id TEXT PRIMARY KEY,
    revision BIGINT NOT NULL DEFAULT 0
);
CREATE TABLE user_preferences (
    owner_user_id TEXT NOT NULL,
    scope TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (owner_user_id, scope)
);
CREATE TABLE user_preference_operations (
    owner_user_id TEXT NOT NULL,
    operation_id TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    result TEXT NOT NULL,
    PRIMARY KEY (owner_user_id, operation_id)
);
