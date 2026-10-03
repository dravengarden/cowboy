CREATE TABLE browser_device_bindings (
    cookie_name TEXT NOT NULL,
    token_hash TEXT NOT NULL,
    public_key TEXT NOT NULL,
    expires_at_ms BIGINT NOT NULL,
    PRIMARY KEY (cookie_name, token_hash)
);
