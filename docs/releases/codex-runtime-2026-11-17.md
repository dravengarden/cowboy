# Codex runtime upgrade — 2026-11-17

Status: source and pins updated on `claude/codex-deps-2026-11-17`. Not yet in a
component release, signed, published or installed; the integrating component
release bumps the Plugin versions.

## Dependency audit

Registry and release data were read from npm and GitHub on 2026-11-17.

| Dependency | Previous | Accepted | Decision |
| --- | --- | --- | --- |
| Codex ACP | 2.5.0 | 2.7.0 (`7c98a160`) | Owned patch rebased (see `plugins/codex/runtime/README.md`) |
| Codex | 0.165.0 | 0.166.0 | Inside ACP 2.7's `^0.166.0`; 0.167.0–0.169.0 wait for an ACP release that declares them |

Both pins are shared by `codex` and `codex-deepseek`. ACP 2.7 moves the Agent
Client Protocol SDK to 2.6.0. Recommended presets and new-session defaults are
unchanged: the bundled model catalog of 0.166.0 is identical to 0.165.0's, with
`gpt-6.1-sol`, `gpt-6-astra` and `gpt-6-luna` and the same reasoning efforts. No
model request was made.

Sources:
[Codex ACP 2.6.0](https://github.com/agentclientprotocol/codex-acp/releases/tag/v2.6.0),
[Codex ACP 2.7.0](https://github.com/agentclientprotocol/codex-acp/releases/tag/v2.7.0),
[Codex 0.166.0](https://github.com/openai/codex/releases/tag/rust-v0.166.0),
[0.167.0](https://github.com/openai/codex/releases/tag/rust-v0.167.0),
[0.167.2](https://github.com/openai/codex/releases/tag/rust-v0.167.2),
[0.168.0](https://github.com/openai/codex/releases/tag/rust-v0.168.0),
[0.169.0](https://github.com/openai/codex/releases/tag/rust-v0.169.0).

## Retained: native Codex 0.169.0

ACP 2.6 and 2.7 declare `@openai/codex ^0.166.0`. Following the established
policy, native Codex stays inside that range; 0.169.0 stays unadopted until an
ACP release declares it. It is ready to adopt then. A candidate built with
0.169.0 passed the same Codex suites, and the app-server TypeScript schemas
that 0.166.0 and 0.169.0 generate (`codex app-server generate-ts
--experimental`) differ only by additions:

- 30 new types, and the 32 changed types only add: new methods
  (model-provider configuration, Guardian review, approved-plan handoff),
  optional request fields (`environments`, `responsesapiClientMetadata`,
  `allowInProgressTurn`), response fields (`userId`, `moderation`,
  `contentTokenCount`) and enum variants (`inferenceHookDenied`,
  `humanCommand`).
- Nothing is removed or renamed. New required fields appear only in
  server-to-client responses and notifications, which ACP reads leniently.
  `humanCommand` approvals are sent only to an
  app-server client that sets `requiredHumanApproval`; ACP does not.
- An offline probe of `thread/start` with two configured MCP servers reports
  the same `mcpServer/startupStatus/updated` sequence on 0.165.0, 0.166.0 and
  0.169.0.

## Upstream changes

| Change | Version | Disposition |
| --- | --- | --- |
| Configured permission profiles become extra session modes; every session reads them | ACP 2.6 | Ordinary sessions follow upstream; Cowboy's codec already reads grouped select options. Managed children skip the reads and offer no profile mode (new patch hunk and test) |
| `thread/sample` follow-up prompt prediction | ACP 2.6 | AIR-only and opt-in; Cowboy is not an AIR client, so no extra model request |
| Async Codex questions shown as form elicitations | ACP 2.6 | Adopted; Cowboy declares form elicitation and presents it through its permission path |
| ACP SDK 2.2–2.6, v2 resume metadata resync, close teardown, interrupt retry fencing | ACP 2.6 | Cowboy speaks protocol v1; resume conformance unchanged (worker suite) |
| Gateway routing provider and vendor-prefixed model IDs | ACP 2.6 | Relevant to `codex-deepseek`; its gateway and launcher are unchanged |
| Delegated tasks as linked `session_message`; `description` becomes the agent role | ACP 2.7 | Cowboy already folds linked messages into the child row (as for Claude) |
| Structured plans as `plan_update` items | ACP 2.7 | Needs `clientCapabilities.plan`, which Cowboy does not declare; unchanged |
| Retry cause in the AIR retry warning | ACP 2.7 | AIR-only |
| `priority` service tier recognised as Fast | ACP 2.7 | Bug fix; no Cowboy preset sets a service tier |
| Blocked or stopped hooks reported as a session notice | ACP 2.7 | Adopted; a hook block is no longer a silent `end_turn` |
| Approval/permission settings preserved across resume and fork; thread startup races | 0.166 | Benefits durable resume; worker cold-resume checks pass |
| GPT-6.1 Terra in the bundled catalog | 0.166 | Present since 0.165.0; no preset change |
| Interrupted turns close cleanly; delayed aborts do not interrupt a later turn | 0.169 | Not yet adopted (outside the ACP range) |
| `codex exec` keeps untrusted project config disabled | 0.169 | Not adopted; not used by Cowboy (ACP app-server) |
| TUI, Cloud, Bedrock/GCP, Windows and macOS changes | 0.166 (0.167–0.169 not adopted) | Not used by Cowboy |

## Owned adapter patch

The patch is rebased onto 2.7.0 with a three-way merge and applies at fuzz 0.
Upstream absorbed no hunk. Two changes go beyond a mechanical rebase:

- A managed read-only child returns from session configuration before
  upstream's new permission-profile listing and effective-configuration read.
  Its turns keep the fixed read-only sandbox policy and never send a profile's
  `permissions`. `CowboyManagedPermissionProfiles.test.ts` pins this and fails
  without the guard.
- The image-profile fixture gains `generationId`, a new required field of the
  generated image item type.

The 33 recorded AIR initialize fixtures again advertise
`idleBehavior: "promptRequired"`, regenerated from upstream's 2.7.0 fixtures.

## Verification

Receipts and logs are under `/srv/storage/fast0/agent/tmp/codex-deps-2026-11-17/`;
the accepted 0.166.0 receipts carry a `-0.166.0` suffix.
Every fixture used scripted loopback APIs and fake credentials; no real model
request was made. `tools/remote_impact.py --base HEAD~1` selected exactly the
Codex suites below.

| Gate | Result |
| --- | --- |
| Codex ACP typecheck and upstream tests with the patch (runtime builder) | 2,546 passed, 39 skipped upstream |
| `codex-worker` (packaged 2.7.0 adapter and 0.166.0 CLI, executor 0.159.3) | Accepted, 34 checks |
| `codex-turn` / `child-stop` / `codex-hooks` on packaged 0.166.0 | Accepted (9 checks / 4 cases / 10 hooks) |
| `codex-image` on packaged 0.166.0 | Accepted, 13 checks |
| Same five suites on a 0.169.0 candidate (retained) | Accepted (34 / 9 / 4 / 10 / 13) |
| `provider-check` (includes `plugin-check`) | Passed on a provisional component release; see below |
| `check-compact` | Passed at `edea0efc` (0.169.0 pins) on the same provisional component release; the 0.166.0 switch changes only pins and docs |

The Codex turn and hooks probes ran under a user and PID namespace whose PID 1
sits outside the probe's network namespace, as on 2026-10-30.

`plugin-check` refuses a changed `cowboy.provider-runtime` digest until a
component release records it. The gates above ran on a disposable local commit
that added such a release; it is not part of this branch.
