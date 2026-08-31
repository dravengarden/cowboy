-- Bind each browser session to the primary authentication method that created
-- it. Existing sessions remain NULL because their source cannot be recovered
-- safely; the next successful primary authentication binds them.
ALTER TABLE user_sessions
    ADD COLUMN primary_auth_method text;

ALTER TABLE user_sessions
    ADD CONSTRAINT user_sessions_primary_auth_method_check CHECK (
        primary_auth_method IS NULL
        OR (
            char_length(primary_auth_method) BETWEEN 1 AND 128
            AND primary_auth_method ~ '^[a-z0-9-]+$'
        )
    );
