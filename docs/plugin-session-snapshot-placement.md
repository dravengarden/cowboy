# Session launch declarations and worker snapshots

The Machine broker owns the accepted launch declaration for detached workers.
A launch-bearing worker snapshot is an observation of that declaration, not a
new placement declaration. Its session ID must match the connected session;
its Provider ID, runtime cwd, system-session flag and execution binding must
match an existing declaration. Malformed bindings remain refused.

Validation and registry replacement share one critical section, using the
existing deletion lock order. An old worker cannot overwrite a declaration
staged for explicit workspace reset. Reset still declares its replacement
before stopping the old worker; `EnsureSession` and reset retain their current
meanings. Provider release/auth generations, native thread materialization,
context budgets and worker generations retain their existing update paths.

Only the original connection and worker epoch may submit an observation or
receive its rejection. A rejected snapshot updates neither accepted worker
state nor the launch registry. The runtime IPC reader stops processing it
before Controller projection, rollout rehabilitation or cutover. A following
real IPC frame is the test synchronization barrier; absence of projection is
not inferred from a delay.

This is a finite Machine broker fence, with no new wire fields or protocol
floor. Launch-less legacy snapshots retain their existing compatibility path.
It does not mint durable Session incarnations, prevent filesystem ABA, grant
worktree authority or establish a general Session state lease. Machine restart
still reconstructs declarations from Controller and surviving worker records;
continuous Machine-owned Session/worktree identity remains open. Filesystem
read observations remain the separate [protocol-25 contract](plugin-session-root-observations.md).
