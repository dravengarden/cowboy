# Operations and growth policy

## Database migrations

Daemon startup may run schema-only, bounded migrations. A migration must not
rewrite an existing unbounded table or perform a data backfill: startup has no
HTTP readiness endpoint until migrations finish, so such work turns a deploy
into an outage.

Controller component rollback restores an older binary without rolling back
PostgreSQL. Every controller therefore ignores already-applied migration
versions newer than its embedded migration set while still verifying every
known checksum. Before the first release that introduces a new migration, the
current rollback target must already contain this compatibility behavior. This
is a deployment bridge, not permission to rewrite a published baseline.

Large transformations use expand/contract:

1. add the nullable column/table/index needed by both old and new code;
2. deploy dual-read/dual-write code;
3. backfill in small committed batches from an explicit maintenance command;
4. verify counts and error metrics;
5. switch reads, then remove the old representation in a later release.

The legacy event-log compaction predates this rule and demonstrated the failure
mode by blocking one startup for more than ten minutes. Do not repeat it.

PostgreSQL and SQLite each have one byte-for-byte immutable consolidated
baseline. Their SQL remains backend-specific; never copy one baseline to the
other and assume it is portable. Every new storage change must update both
histories and pass the shared storage contract against both backends.

## Backups

The NixOS service owns a daily custom-format `pg_dump` timer. Each dump is
checked with `pg_restore --list` before publication and retained for 14 days in
`/var/backup/cowboy`. A quarterly restore into a temporary database is the
operator-level recovery drill; listing a dump only proves archive readability,
not full restore semantics.

SQLite deployments must create backups through SQLite's online backup API or
`VACUUM INTO` while the controller is running. Copying only the main database
file can omit committed WAL contents. A backend backup and the artifacts tree
form one recovery set.

## Artifacts

Durable history externalizes large ACP image blocks into the content-addressed
`$COWBOY_DATA_DIR/artifacts` directory. Live prompts remain inline ACP blocks,
so providers see the original protocol. History stores immutable HTTP URLs.
Back up this directory together with the selected database; database-only restores retain
the transcript but cannot render externalized images.
