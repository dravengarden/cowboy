# Codex resume transport

Codex 3.1.18 restores an existing native thread without requesting its display
history. It keeps the native CLI, authentication contract, restore deadline, and
native storage format unchanged.

## Source ownership

[`third_party/codex-acp`](../../../third_party/codex-acp) pins the ACP release,
its source archive and npm lock by digest, and carries Cowboy's changes as a
series of source deltas, one per feature, managed with the generic
[third-party workflow](../../../docs/third-party.md). Each delta's commit
message records its purpose, upstream state, drop condition and tests.
`build.ts` prepares the patched tree, runs the upstream type check, complete
test suite and build, then packages the result with the existing pinned Node
and native CLI artifacts. Both supported platforms carry the concatenated
series (`codex-adapter.patch`), the source identity, each delta's digest and
the resulting bundle digest in `codex-source.json`. Runtime artifacts are
immutable and signed through the ordinary Plugin lifecycle. Failed builds
remove the candidate runtime binding. `codex-deepseek` uses the unmodified
upstream release.

The shared runtime component remains unchanged. This Provider owns the changes
(delta IDs in parentheses):

- ACP `session/resume` sends native `thread/resume` with `excludeTurns: true`.
  Upstream has done this since ACP 1.11; no delta remains.
- (`jsonl-frame-reader`) The JSONL reader searches incoming bytes once, preserves split UTF-8
  sequences, and assembles each completed frame once. The compatibility ceiling
  is 256 MiB per frame; a malformed, oversized or incomplete frame stops the
  native process, whose exit closes pending requests, with a diagnostic that
  does not include conversation contents.
- (`native-launch-arguments`) The Provider launcher passes its validated leading `-c` options to the native
  client, which launches the exact bound `CODEX_PATH` directly. No intermediary
  executable owns or rewrites the native protocol.
- (`server-overloaded-failure`) A terminal `serverOverloaded` error ("Selected model is at capacity") fails
  the prompt with its native error data, as upstream already does for
  `usageLimitExceeded`, and keeps the readable message in the transcript. A
  managed call then reports `provider_error` instead of treating the message as
  model output such as an invalid structured review.
- (`steer-prompt-required`) `initialize` advertises steering
  `idleBehavior: "promptRequired"`, and an opted-in host may replay a steer
  only after an explicit native no-active-turn refusal.
- (`managed-read-only-profile`, `native-image-profile`) The managed read-only
  and image profiles; see [native capabilities](../../../docs/managed-native-capabilities.md).
- (`configured-mcp-startup-wait`) The MCP startup wait below.
- (`native-codex-version`) The adapter declares the native Codex release this
  Provider packages (`@openai/codex ^0.169.1`) ahead of upstream's
  `^0.166.0`, so upstream's suites run against it; see the 2026-11-18 audit.

## Authentication default

codex-acp logs in with `DEFAULT_AUTH_REQUEST` whenever the native app-server
reports that a login is required, and answers ACP `authRequired` only when no
default is set. Its API-key method reads `CODEX_API_KEY` or `OPENAI_API_KEY`
from the adapter environment. Package 3.5.14 and earlier declared
`{"methodId":"api-key"}` for every launch, so a ChatGPT account whose login
expired failed `session/resume` with the internal error "CODEX_API_KEY or
OPENAI_API_KEY is not set" on every worker generation instead of reaching
Cowboy's reconnect state.

The launcher now declares the API-key request only when the projected
credential is an API key (the `api-key` method projects `OPENAI_API_KEY`), and
drops a declared API-key request that has no key. Any other explicit request,
such as the offline conformance fixtures' `chat-gpt`, is kept. A ChatGPT
account then gets ACP `authRequired`; the worker maps it to the typed
"Provider authentication required" detail, the Controller marks the projected
account generation expired, and the Machine does not retry an older worker
generation. Reconnecting the same account in Settings retries the retained
native thread. `codex-deepseek` uses its own gateway login and the shared
launcher; it never declared this default and is unchanged.

## MCP startup wait

The worker sends `_meta.mcpStartupAwaitTimeoutMs` (15 s) on `session/new`,
`session/resume` and `session/fork` for the Codex family. Upstream codex-acp
honors it only for servers in the request's `mcpServers`. Every server a Cowboy
Codex session uses comes from native configuration instead: user/project
`config.toml`, or the launcher's `-c mcp_servers.*` options.

The `configured-mcp-startup-wait` delta extends the wait to the servers the Provider's native bridge
injects. The launcher computes their exact names with the bridge's own naming
rules (`injectedMcpServerNames` in `memory.mjs`): the session Cowboy MCP
(`cowboy`, or a fresh `cowboy_session*` name when native configuration already
defines `cowboy`) for executor or call-context sessions, and Matrix when its
backend tools are enabled. It passes them as a JSON array in
`COWBOY_CODEX_AWAIT_MCP_SERVERS`. Inherited values are removed and only an
ordinary launch with the bridge declares any; managed profiles never do, and
the adapter also ignores the list under a managed profile.

When the metadata asks for a positive wait, `tryCreateSession` adds those names
to the servers `McpSessionStartup` awaits, with the upstream tracker semantics:
events for the thread or global events, after the version read before the
native request, bounded by the requested timeout. Each failed or cancelled
server is reported as an `mcp__<server>__startup` tool call; a fork waits but
publishes nothing. Without the metadata, without names, and for `session/load`
nothing changes. User-configured servers are not awaited.

Native Codex `0.165.0` reports natively configured servers on the thread. An
offline probe of the packaged CLI with two `-c mcp_servers.*` servers (one
valid, one missing executable) received `mcpServer/startupStatus/updated` with
the thread ID for both servers, `starting` then `ready` or `failed`, on
`thread/start` and on `thread/resume`, before the resume response. Native
`0.166.0` and `0.169.0` report the same `thread/start` sequence as `0.165.0` in
the same offline probe.

The servers stay native configuration rather than ACP `mcpServers`, because
ACP cannot express them without changing their meaning:

- ACP stdio maps only to `{command, args, env}` and ACP HTTP only to
  `{url, http_headers}`, with literal values. Session Cowboy MCP depends on
  `environment_id` and `cwd` to run `cowboy mcp` on the session's execution
  target (the keeper then supplies its own call context), on
  `env_vars=["COWBOY_CALL_CONTEXT"]` locally, and on `tool_timeout_sec=75`.
  Matrix memory depends on `env_http_headers`, which keeps its bearer token in
  the process environment rather than in configuration, and on
  `enabled_tools`, which exposes only the tools the backend declares.
- These servers are assembled inside the native bridge (`memory.mjs`), on the
  native app-server stream. The Rust worker that sends the ACP request knows
  none of them.
- codex-acp skips, and does not await, a requested server whose name any
  native configuration layer already defines.

The launcher and the bridge each list the native servers once, from the same
arguments; a configuration change between those two reads can only make the
wait miss a server or end at its timeout.

## Dependency audit (2026-11-18)

Native Codex moves to `0.169.1` ahead of ACP. `0.169.1` is the first release
whose bundled model catalog carries `gpt-6.2-sol` and `gpt-6.2-astra`; it
lists `gpt-6.1-sol` and `gpt-6-astra` as the previous generation. ACP `2.7.0`
is still the latest stable release, and its newest preview
(`v2.7.1-preview.2`) still declares `@openai/codex ^0.166.0`, so the
`native-codex-version` delta declares `^0.169.1` until an upstream release
does. The ACP pin is unchanged.

- The app-server TypeScript schema of `0.169.1` (`generate-ts
  --experimental`) removes no type, field or string literal of `0.169.0`'s,
  which in turn only added to the `0.166.0` schema ACP 2.7 was generated from.
  The only new required fields in a client-to-server or server-request type
  belong to a new MCP elicitation mode, `openai/userVerification`. ACP routes
  no such mode to ACP elicitation or its permission fallback, so it is not
  presented; Cowboy configures no MCP server that sends it.
- The offline MCP startup probe reports the same `thread/start` sequence on
  `0.166.0`, `0.169.0` and `0.169.1`.
- Type checks and all 2,548 enabled upstream tests pass with the series (39
  remain skipped upstream). The new delta's test pins the declaration, the
  lockfile and the installed package.

Recommended presets follow the new generation: Sol 6.2 Medium (the new
default and new-session default), Astra Medium and Astra Max on
`gpt-6.2-astra` (their IDs, used by managed calls, are unchanged), Sol 6.2 Max
and the unchanged Luna 6 Max. See
[the release record](../../../docs/releases/codex-0169-2026-11-18.md).

## Dependency audit (2026-11-17)

The pin moves to ACP `2.7.0` at
`7c98a1608022cbb2b7e425ae7a1df5fde86c6ff2` with native Codex `0.166.0`. ACP
2.6 and 2.7 declare `@openai/codex ^0.166.0`, which admits only 0.166.x, so
native `0.167.0` to `0.169.0` stay unadopted until an ACP release declares
them. The bundled model catalog of `0.166.0` equals `0.165.0`'s, so every
Recommended preset (`gpt-6.1-sol`, `gpt-6-astra`, `gpt-6-luna` with their
efforts) remains valid. The Codex worker, turn, child-stop, hooks and managed
image suites pass with the packaged `0.166.0` CLI; see
[the release record](../../../docs/releases/codex-runtime-2026-11-17.md).

Native `0.169.0` is ready to adopt once ACP declares it:

- The app-server TypeScript schema generated by `0.169.0` (including
  experimental methods) only adds to the `0.166.0` schema ACP 2.7 was generated
  from: new methods, notifications, optional request fields, response fields
  and enum variants. Nothing is removed or renamed. The new `humanCommand`
  approval kind is sent only to clients that opt in with
  `requiredHumanApproval`, which ACP does not.
- Its bundled model catalog is identical to `0.166.0`'s.
- The same Codex suites passed with a packaged `0.169.0` candidate built from
  this adapter source.

ACP 2.6 exposes permission profiles from native configuration as extra session
modes, grouped under "Permission Profiles", and reads the effective
configuration and profile list for every new, resumed and forked session. The
rebased patch keeps a managed child out of both: it returns before those reads,
offers no profile mode, and its turns keep the fixed read-only sandbox policy
instead of a profile's `permissions`. A new Cowboy test pins this. ACP 2.7
generates `generationId` on image items; the Cowboy image fixture adds it. All
other hunks apply in meaning unchanged: initialize's `idleBehavior`
advertisement, the lost-steer rule, managed reconfiguration and title gates, the
MCP startup wait, the JSONL reader, the `serverOverloaded` failure and the image
profile.

Behavior Cowboy receives from the new adapter:

- Delegated tasks of standard subagent sessions arrive as linked
  `session_message` updates, and a subagent's `description` is its role rather
  than the task prompt. Cowboy already folds linked messages into the child's
  row, as it does for Claude.
- Async Codex questions (`request_user_input_async`) become form elicitations
  while their turn runs; Cowboy presents them through its existing elicitation
  permission path.
- A hook that blocks or stops Codex now produces a session notice naming it
  instead of an unexplained `end_turn`.
- AIR-only additions (`thread/sample` prompt prediction, structured
  `plan_update` items that need `clientCapabilities.plan`) stay off: Cowboy
  declares neither.

Type checks and all 2,545 enabled upstream tests pass with the patch (39 remain
skipped upstream). Authoritative sources:
[ACP 2.6.0](https://github.com/agentclientprotocol/codex-acp/releases/tag/v2.6.0),
[ACP 2.7.0](https://github.com/agentclientprotocol/codex-acp/releases/tag/v2.7.0),
[Codex 0.166.0](https://github.com/openai/codex/releases/tag/rust-v0.166.0),
[Codex 0.167.0](https://github.com/openai/codex/releases/tag/rust-v0.167.0),
[Codex 0.168.0](https://github.com/openai/codex/releases/tag/rust-v0.168.0),
[Codex 0.169.0](https://github.com/openai/codex/releases/tag/rust-v0.169.0).

## Dependency audit (2026-10-11)

The `image_v1` managed profile delegates native image generation/editing through
ACP. It requires supported ChatGPT authentication, retains read-only execution,
and keeps signed restrictions in thread configuration as well as process
arguments. Generated image content appears once in ACP; Cowboy validates and
delivers it as an artifact. Managed calls skip the extra model-generated title.
See [native capabilities](../../../docs/managed-native-capabilities.md) for
ownership, limits, fallback and acceptance evidence.

The pin moves to ACP `2.5.0` at
`34e08970aa324a2be985f5080674d5aa91d7666f` with native Codex `0.165.0`. ACP
2.5 declares `@openai/codex ^0.165.0`, so native `0.166.0` stays unadopted
until an ACP release declares it.

ACP 2.4 and 2.5 add standard subagent sessions (only for clients that declare
`clientCapabilities.subagents`, which Cowboy does not), AIR-only prompt
suggestions and batch session lookup, and move to ACP SDK 2.1. Upstream now
parses the steering `idleBehavior: "promptRequired"` opt-in itself (#593), so
the rebased patch drops its own request parsing and response types and keeps:

- advertising `idleBehavior: "promptRequired"` in `initialize`, which upstream
  does not, because Cowboy admits current-turn steering only on that signal;
- the ambiguous-error rule: a host that opted in may replay only an explicit
  native no-active-turn refusal, never a lost reply;
- the managed-profile reconfiguration refusals on upstream's new
  `getSessionState`, and the managed title suppression around the new
  turn-completion path.

Type checks and all 2,222 enabled upstream tests pass with the patch (40
remain skipped upstream). Authoritative sources:
[ACP 2.4.0](https://github.com/agentclientprotocol/codex-acp/releases/tag/v2.4.0),
[ACP 2.5.0](https://github.com/agentclientprotocol/codex-acp/releases/tag/v2.5.0),
[Codex 0.165.0](https://github.com/openai/codex/releases/tag/rust-v0.165.0).

### ACP 2.3.0 (2026-10-28)

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
