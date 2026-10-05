# Remote execution parity fixes — 2026-10-05

Claude Code Plugin 3.4.8 is active on OVH. Source revision
`8aae372fcf0801e482000b0a8ba4f85cca3164a9` fixes three observed differences in
remote execution:

- Split UTF-8 characters survive independent stdout/stderr chunks, result
  boundaries and cold resume. Invalid final bytes retain replacement semantics.
- A failed output read or state-file replacement does not advance the retained
  output cursor and skip bytes on retry.
- Interrupt includes foreground starts awaiting their remote acknowledgement.
  Termination requests for existing foreground jobs do not wait for pending
  starts; each pending start is terminated by its original identity after
  acknowledgement, without replaying the command.

The native versions remain Claude CLI 2.1.287, ACP 0.84.0, Node 24.21.0 and
executor Codex 0.159.3. This release introduces no additional tool restrictions.
Codex Plugin 3.3.2 remains installed unchanged.

## Acceptance and activation

The complete `just check-compact` gate passed after integration with current
main. The owned source gate passed 58 tests, including regressions that failed
before the output and pending-start fixes. Native Codex review found no concrete
new defect. Exact packaged Claude execution passed 28 checks through the real
worker/keeper and a scripted API, including a new split-stream UTF-8 case.
Those checks do not include the optional enrolled Matrix suite or real-model
inference. The separate [candidate evidence](../experiments/remote-parity-candidate-2026-10-05.json)
retains exact source hashes, scenarios and limitations.

Actual Linux x86_64 and macOS arm64 artifact probes passed. Worker initialization,
old/new generation coexistence and descendant drain passed. An interrupted Mac
artifact transfer was rejected by the digest check; a verified continuation
completed before the successful probe. The clean-commit rebuild matched the
accepted package and all runtime digests exactly.

Current, next-transaction recovery and cold Controller readers accepted the
candidate. Full prospective Catalog and actual host-policy preflights passed
for both distinct reader binaries. The snapshot includes both the canonical
and retained legacy Catalog roots; testing only the canonical root initially
omitted the existing authentication Plugin and was correctly rejected.
Production signing was independently checked against the existing trusted
publisher key, and all five public artifact URLs returned the expected bytes.

Signed release digest:
`sha256:485d690dcb0c3e1a7612a6772fa8683941e540a5fdec56da35c69b248a74cfe6`.
Normal Operator operation `ovh-claude-remote-parity-3-4-8-20261005` returned HTTP
204 and an Applied Machine receipt:
`installation-e5b7e74e20643468cb5776d427145688280757e16e6ccde7ac458fe30b7f1d4d`.
Subsequent inventory confirmed active 3.4.8 with current credential replica and
materialization. Version 3.4.7 remains the rollback generation. No Controller,
resident Machine or live session was restarted for this release.
The [release receipt](../acceptance-results/remote-parity-2026-10-05.json)
records the exact publication and installation evidence.

## Remaining parity work

This release is not a declaration that all native behavior is equivalent.
The [full audit](../remote-tools-coverage-audit-2026-10-05.md) covers CodeAct,
agents, permissions, hooks, skills, MCP/browser placement, files/artifacts,
background tasks, cancellation, reconnection and lifecycle failures.

Claude's native background notification path has a promising shell-prefix
entry point, demonstrated through a separate executor on the same host; it
still needs a production cross-host bridge preserving shell snapshots, native
task ownership, cancellation and recovery. Native agents/skills, permission
modes and target-dependent hook discovery require separate implementation and
acceptance. A subsequent test-only follow-up verifies native Codex CodeAct
patch/shell/image routing, parallel error results, yield/wait and cold resume
against the installed 3.3.2 package. It also corrects sandbox-helper packaging
and setup-capability handling in the fixture. No additional production update
is needed for these test changes. See the
[native CodeAct evidence](../experiments/codex-native-codeact-2026-10-05.json).
This does not establish arbitrary nested-agent or project-hook parity, or
pending-cell recovery after native-runtime death.
Unverified behavior stays explicitly identified; no capability was newly
disabled to make this audit pass.
