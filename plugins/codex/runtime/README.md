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
  is 256 MiB per frame; a malformed, oversized or incomplete frame stops the
  native process, whose exit closes pending requests, with a diagnostic that
  does not include conversation contents.
- The Provider launcher passes its validated leading `-c` options to the native
  client, which launches the exact bound `CODEX_PATH` directly. No intermediary
  executable owns or rewrites the native protocol.
- A terminal `serverOverloaded` error ("Selected model is at capacity") fails
  the prompt with its native error data, as upstream already does for
  `usageLimitExceeded`, and keeps the readable message in the transcript. A
  managed call then reports `provider_error` instead of treating the message as
  model output such as an invalid structured review.

## Dependency audit (2026-10-11)

The `image_v1` managed profile delegates native image generation/editing through
ACP. It requires supported ChatGPT authentication, retains read-only execution,
and keeps signed restrictions in thread configuration as well as process
arguments. Generated image content appears once in ACP; Cowboy validates and
delivers it as an artifact. Managed calls skip the extra model-generated title.
See [native capabilities](../../../docs/managed-native-capabilities.md) for
ownership, limits, fallback and acceptance evidence.

The pin moves to ACP `2.3.0` at
`344a520632e4c8c02f781a7a20490dfac52d4d0a` with native Codex `0.164.0`. ACP
2.3 declares `@openai/codex ^0.164.0`, so native `0.165.0` stays unadopted
until an ACP release declares it.

ACP 2.3 moves to ACP SDK 2.0 and adds a second protocol chain: each connection
keeps the version of its first `initialize`, and the v1 chain Cowboy speaks is
unchanged except for upstream bug fixes. The owned patch is rebased onto it:

- Imports follow the SDK's `unstable` entrypoint; Cowboy's own modules are
  unchanged.
- The steering `idleBehavior: "promptRequired"` capability now lives in the
  shared `initializeExtensionsMeta`, so v1 and v2 initialization advertise it
  alike. The late-steer refusal and ambiguous-error rule keep their meaning
  around upstream's new `clientUserMessageId` and running-turn check.
- Session configuration keeps upstream's new unsupported-skill filter beneath
  the managed profile's project-trust omission; a managed turn keeps its
  output schema beside the new client message identity.
- Upstream's new `/review` turn tests used `serverOverloaded` as an ordinary
  error. Cowboy fails the prompt on it, so those agent-text and fatal-wait
  cases use `internalServerError` instead.

Type checks and all 1,937 enabled upstream tests pass with the patch (36
remain skipped upstream). Authoritative sources:
[ACP 2.3.0](https://github.com/agentclientprotocol/codex-acp/releases/tag/v2.3.0),
[Codex 0.164.0](https://github.com/openai/codex/releases/tag/rust-v0.164.0).

### ACP 2.2.2 (2026-10-18)

The pin moved to ACP `2.2.2` at
`202e66e63343ec4f5a5e35ab590251d813d129ca` with native Codex `0.160.1`. ACP
2.2 declares `@openai/codex ^0.160.1`; native `0.161.0` to `0.162.1` are
outside that range and stay unadopted until an ACP release declares them.

ACP 2.2 adds an app-server supervisor. It reads what the process wrote before
it died, then disposes the connection itself, and can start a new app-server.
It learns that the output is drained from a marker the reader delivers after
the last frame. The owned patch is rebased onto that lifecycle:

- The reader keeps bounded frames, strict UTF-8 and redacted failures, and a
  malformed, oversized or incomplete frame still ends the stream instead of
  being skipped. It no longer closes on a clean end of output, and it keeps
  delivering the supervisor's marker after an end or a failure.
- A reader failure stops the native process instead of disposing the
  connection. The exit then disposes the connection and rejects pending
  requests, through the supervisor when one owns the process (after at most
  its one second close grace) and directly otherwise. No new request starts on
  the failed stream.
- The validated private `-c` options precede the startup arguments ACP now
  passes (`app-server` and, for client hooks, its own `-c hooks=` override).
  The execution bridge receives the same startup arguments.

The managed read-only profile hunks apply unchanged in meaning; ACP's session
configuration now also returns the MCP servers it skipped, which a managed
session reports as none. Type checks and all 1,504 enabled upstream tests pass
with the patch (36 remain skipped upstream).

Authoritative sources:
[ACP 2.2.2](https://github.com/agentclientprotocol/codex-acp/releases/tag/v2.2.2),
[Codex 0.160.1](https://github.com/openai/codex/releases/tag/rust-v0.160.1).

## Dependency audit (2026-09-30)

The candidate pins ACP `2.0.1` at
`7a8e00fe46b299264f5ebe9f250288636f1485cc` and native Codex `0.159.2`.
Upstream ACP 2 introduces the AIR tool-call contract and exact diff patches;
its Codex range includes this native patch release. Source type checks, the
complete upstream tests and Cowboy worker/resume checks remain release gates.

Upstream now has a linear UTF-8 line splitter. The owned patch retains bounded
frames, strict UTF-8 validation, redacted parse failures and explicit disposal
of pending requests on stream closure. It preserves upstream's valid final
JSON frame without a trailing newline. Malformed frames remain fatal instead
of being skipped: dropping a response can otherwise leave a resume pending.
Upstream Unicode, final-frame and linear-time tests remain; separate Cowboy
tests cover malformed/oversized/incomplete frames and native process cleanup.

Authoritative sources:
[ACP 2.0.1](https://github.com/agentclientprotocol/codex-acp/releases/tag/v2.0.1),
[Codex 0.159.2](https://github.com/openai/codex/releases/tag/rust-v0.159.2).

## Dependency audit (2026-09-25)

The pin moves to ACP `1.13.1` at
`b1b8490cd165c18626dc3fe83836cdacdef94cd3` with native Codex `0.156.1`. The two
are a matched upstream pair: `1.13.1` exists only to pin `0.156.1`. Upstream
still assembles JSONL frames from an accumulated string, so the owned reader
patch is retained and applies to that exact revision unchanged at fuzz 0. The
`session/resume` hunk stays absorbed upstream. `1.12.0` adds tool names to ACP
tool-call events and derives file-change reports and diff statistics from turn
diffs; `1.13.0` adds experimental session notices and ACP compaction updates and
prefers terminal output deltas. Both are compatible with this patch.

Authoritative sources:
[ACP 1.13.1](https://github.com/agentclientprotocol/codex-acp/releases/tag/v1.13.1),
[Codex 0.156.1](https://github.com/openai/codex/releases/tag/rust-v0.156.1).

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
