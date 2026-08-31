-- Bind each browser session to the primary authentication method that created
-- it. Existing sessions remain NULL because their source cannot be recovered
-- safely; the next successful primary authentication binds them.
ALTER TABLE user_sessions
    ADD COLUMN primary_auth_method TEXT CHECK (
        primary_auth_method IS NULL
        OR (
            length(primary_auth_method) BETWEEN 1 AND 128
            AND primary_auth_method NOT GLOB '*[^a-z0-9-]*'
        )
    );
