# Codex resume transport

Codex 3.1.18 restores an existing native thread without requesting its display
history. It keeps the native CLI, authentication contract, restore deadline, and
native storage format unchanged.

## Source ownership

`source.json` pins the ACP source archive and npm lock by digest.
`adapter.patch` is a source patch against that exact revision, not a
modification to an installed bundle. `build.ts` runs the upstream type check,
complete test suite and build, then packages the result with the existing pinned
Node and native CLI artifacts. Both supported platforms carry the patch, source
identity and resulting bundle digest in `codex-source.json`. Runtime artifacts
are immutable and signed through the ordinary Plugin lifecycle. Failed builds
remove the candidate runtime binding.

The shared runtime component remains unchanged. This Provider owns the changes:

- ACP `session/resume` sends native `thread/resume` with `excludeTurns: true`.
  Explicit history loading retains its existing behavior.
- The JSONL reader searches incoming bytes once, preserves split UTF-8
  sequences, and assembles each completed frame once. The compatibility ceiling
  is 256 MiB per frame; malformed, oversized or incomplete frames close pending
  requests immediately with a diagnostic that does not include conversation
  contents.
- The Provider launcher passes its validated leading `-c` options to the native
  client, which launches the exact bound `CODEX_PATH` directly. No intermediary
  executable owns or rewrites the native protocol.

## Dependency audit (2026-09-12)

The retained baseline is ACP 1.10.0 at
`061f9a4a2e463a220d7a3ab2ae5e9732837085ef`, native Codex 0.153.4 and the Node
pins in the shared runtime lock. ACP 1.11.0 already requests `excludeTurns` on
resume but retains the accumulated-string JSONL reader. Its other feature and
protocol changes are outside this compatible incident release. Native 0.154.0 is
likewise not required to fix this transport failure. Both are classified
`no-upgrade` for this release; test a future dependency upgrade independently.

Authoritative sources:
[ACP 1.11.0](https://github.com/agentclientprotocol/codex-acp/releases/tag/v1.11.0),
[Codex 0.154.0](https://github.com/openai/codex/releases/tag/rust-v0.154.0).

## Acceptance

Run the repository's Plugin and Provider gates and the source builder. The patch
includes reader failure, fragmented Unicode, large-frame and pending-request
closure tests. Use the packaged entrypoint for incident acceptance:

```sh
just codex-resume-conformance \
  /absolute/adapter/bin/codex-acp /absolute/native/bin/codex \
  /absolute/rollout.jsonl /absolute/new-receipt.json
```

This Linux test copies the rollout into a private home and isolates networking.
It uses fake authentication, never sends a model turn, and checks cold
projection creation followed by a warm restart. It requires the original native
identity, the actual configured native command, `excludeTurns`, bounded ACP
output, and no history replay or new-thread request. Private temporary logs are
removed; the create-only receipt contains hashes, timing and assertions, without
prompts. This proves restore behavior, not the contents of a subsequent
inference request.

The native media-reference redesign and its separate acceptance requirements are
recorded in [the architecture decision](../../../docs/codex-durable-resume.md).
