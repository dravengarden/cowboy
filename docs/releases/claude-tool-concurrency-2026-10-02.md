# Claude remote tool scheduling, 2026-10-02

Claude Code Plugin **3.2.1** is installed and active on OVH. The source revision
`72d7848ca6957ff86d20c25676d1b6491b626f8a` is published on Cowboy `main`.
The native CLI remains 2.1.287, subscription authentication retains generation
50, and shared component release 3.35.0 and sibling Plugins are unchanged.
The [production receipt](claude-tool-concurrency-2026-10-02.json) binds the exact
release, installation and verification evidence.

## Behavior

The previous bridge serialized every tool, including independent file reads,
searches and task cancellation, behind one queue. It also omitted MCP read-only
annotations, so Claude's native scheduler serialized those tools before they
reached the bridge. Removing only the bridge queue did not solve native dispatch.

ReadFile, GlobFiles and GrepFiles now declare `readOnlyHint: true`, following
the [official native scheduling contract](https://code.claude.com/docs/en/agent-sdk/agent-loop).
Only those tools make this claim. Bash, file mutations and task controls keep
their native sequential classification. The bridge orders file reads and
mutations by normalized path, preserving read stamps while permitting independent
paths to proceed. Each process has a separate output collection queue, so its
cursor cannot be consumed twice. TaskStop submits termination before joining
that collection queue; native user interruption remains supported.

## Verification and activation

- Full `just check` and final `provider-check` passed, including 15 focused
  Claude tests. Regressions cover independent reads, same-file read/edit order,
  cancellation during output collection and single consumption of task output.
- The final immutable packaged CLI/ACP passed all 26 execution-worker checks.
  Two native searches read distinct FIFO fixtures whose content is supplied only
  after both readers open. This proves actual overlapping native/bridge/target
  execution, without relying on timing thresholds. An earlier attempt using
  arbitrary Bash exposed native sequential dispatch and is not acceptance evidence.
- Actual detached old/new workers passed coexistence and descendant drain.
  The exact release passed active, next-transaction recovery and cold Controller
  Catalog checks. Five public URLs matched SHA-256, covering 307,998,050 bytes.
- Installation operation `ovh-claude-3-2-1-parallel-tools-20261002` exceeded its
  initial 90-second observation deadline. After an observed Machine disconnect,
  same-ID reconciliation found the original Applied receipt and completed the
  transaction without repeating installation. The slot has no remaining fence.
- Controller, OVH Machine and existing worker process identities/start times
  were preserved; health is normal. No core component or NixOS activation occurred.

The release digest is
`sha256:449b92180f08638223d95ef2ba1084e9996eca497cfb0422cc484fe462de8db5`;
installation revision is
`installation-5d60052995d00cc5c60bff3027e1a93b5d5686ed23da97e7d8e0e4635ec99e13`.

## Limits

This accepts concurrent read-only dispatch, not production model token/turn or
cross-host latency savings. Verification uses scripted loopback model responses,
not a real subscription inference request. Whole-file transport, non-atomic
conflict checks against unrelated writers, startup guidance snapshots and the
earlier native-feature limits remain. Existing sessions keep their generation
and execution placement; legacy Matrix sessions still use mx. New bound remote
sessions use the newly installed default. No physical device or macOS native
execution acceptance is claimed.
