# Matrix shared memory release — 2026-10-03

Standard Codex and Claude Provider adapters now use the separately versioned
Matrix service when explicitly enrolled. New OVH sessions on registered
Hawk/Falcon projects receive shared long-term recall, automatic completed-turn
learning, evidence-backed corrections and logical forgetting. Native memory
generation/use is disabled for these enrolled sessions; native conversation
history, compaction and project guidance retain their normal owners.

## Implementation and accepted releases

The shared client provides exact project/executor bindings, bounded recall,
HTTP MCP and a private durable outbox. Codex's app-server bridge and Claude's
input bridge retrieve before user turns. Claude's cached project context is
not used for dynamic memory. Memory calls stay on OVH; project tools retain
their target execution connection. Grok and isolated Provider variants were
not enrolled or changed for this integration.

Runtime source `c4e7d418aa81ffa2cce6820e11c535ddde2f2799` passed the full
`just check-compact` gate. Integration revision `3eab9425` includes concurrent
main changes; the resulting Web typecheck and all 2,004 Web tests passed.
Native dependency pins and authentication were unchanged.

| Provider | Version | Signed release digest |
| --- | --- | --- |
| Codex | 3.3.1 | `sha256:d5b985f3373b199c68e83b813004fee80c2312ad686c69e1fdf6021d03c67fe4` |
| Claude Code | 3.4.3 | `sha256:23eb04e31024aae477ccc71cb3111fd34f8cb21646ff99ea31ce0d3b9cd460f1` |

The actual Linux and macOS runtimes passed native dependency probes. Linux
worker generation coexistence and stop/drain passed. Active, next-transaction
recovery and cold Controller readers accepted both release shapes. All ten
published package/runtime URLs were fetched and their exact digests verified.
The live signed Catalog accepted both releases.

The normal host Operator installed both on OVH, using one operation ID per
release and reconciling durable receipts after the observation timeout:

- `ovh-matrix-codex-3-3-1-1790563075662`: completed, Applied
  `installation-e199b6e6f7f2d727ca33381c23319a42dfac7c65865d1b1b038bdbefd5be730f`.
- `ovh-matrix-claude-3-4-3-1790563075662`: completed, Applied
  `installation-e736a9e444db7dd5aca423cb508179cc1747d7b4211ac231ac532592b14d3702`.

Final inventory reported those exact active generations with current credential
replicas/materializations. Neither Provider required further reconciliation.
The initial Codex 3.3.0 staging attempt exposed a version-probe defect under
Matrix enrollment. The corrected 3.3.1/3.4.3 releases bypass session-only memory
binding during artifact inspection. The failed operation was resolved through
normal no-effect reconciliation; historical 3.3.0/3.4.2 publications remain
immutable and are not accepted rollout pins.

## Instance and process boundaries

Matrix 0.1.1 runs from product revision
`55eaa3f22543489ef13c20c933e5fa6de34c52d2`, zipapp SHA-256
`f4628e58dbc4dd23d1c727721273b6cc1b12dde1036f76d9fedc36dab0bd88a8`.
The private `/home/ubuntu/matrix-ovh` repository is separate from the product.
The loopback server and background learning/checkpoint worker are enabled user
services. Each standard Provider has 55 explicit Hawk/Falcon project mappings.

The worker made a real periodic Git commit. Final cleanup checkpoint
`27a8bc76f695f7a2c349577215ccd3c82cf881d4` records sequence 5 with a clean tree.
The instance has no Git remote and push is disabled; this is local persistence,
not off-host backup. Raw observations, credentials and pending jobs are outside
Git. Logical forget retains historical commits.

Controller PID 1998362 and OVH Machine PID 394847 were unchanged, both with zero
restarts. Three pre-existing OVH workers were retained. Only Matrix's own
services restarted for the 0.1.1 upgrade. No Controller, Machine, Web, iOS or
Columbus release was activated by this task. Legacy OVH Matrix was preserved.

## Acceptance and limits

The [machine-readable evidence](../acceptance-results/matrix-memory-2026-10-03.json)
separates the following results:

- Matrix: 33 service tests, pinned Nix build and 12 real-extractor synthetic
  quality checks. Older observed evidence and assistant-only reinforcement
  regressions failed before the 0.1.1 correction and passed afterwards.
- Exact packaged native adapters: four Matrix HTTP MCP/recall/capture/probe
  checks, 20 Codex remote-worker checks and 32 Claude checks, including cold
  resume, actual Claude compaction and retained target execution.
- Real OVH inference: Codex taught a synthetic fact; a fresh Claude recalled
  it. Claude corrected it; a fresh Codex recalled the new value. After logical
  forget, both fresh clients answered `UNKNOWN`. All six turns used automatic
  recall/capture without tool calls. Test records were forgotten and all seven
  acceptance learning jobs completed.

The real inference check used the exact installed adapters with normal owned
authentication and a temporary local OVH project binding. It was not a new
Cowboy-managed cross-host session. The additional product CLI check lacked a
normal device credential and could not create a session; no user credential
was copied and no authentication bypass was used. The separate remote-worker
checks used real native clients and scripted model APIs in an isolated network.

These results establish the integration's tested mechanism, not superiority
over native memory, representative task-quality or token-cost measurements,
standalone hook compatibility, or lossless capture of a killed unfinished turn.
Existing workers retain their old generation. Unmapped local workspaces require
explicit enrollment; other runtime hosts and isolated variants were not cut
over. Grok integration remains deferred.

See [Provider configuration](../matrix-memory.md), the
[ownership boundary](../architecture/08-memory.md), and Matrix's
[architecture and evolution contract](https://github.com/dravengarden/matrix/blob/main/docs/shared-memory.md).
Future policy upgrades require fixed baselines, holdouts, operational cost and
migration/rollback evidence; paper or vendor scores alone are insufficient.
