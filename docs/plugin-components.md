# Plugins and shared components

Cowboy organizes independently versioned integrations as plugins and reusable
implementation as components. The machine-readable sources of truth are
`plugins/*/plugin.json`, `components/plugin-contract/schema.json`,
`components/plugin-contract/authentication-provider.schema.json`,
`components/code-intelligence/schema-v2.json`, and `components/registry.json`.

The target [spatiotemporal design, section 2.3](plugin-spatiotemporal-design.md)
places component libraries within one cross-Service/Machine composition model:
pure utilities, core implementations, authoring contracts, private Plugin
implementations and templates. Owned units do not acquire an installation
identity. The schema-3 dependency-closure release gate and state-store's typed,
owned persistence implement the first component slice of sections 9–10.
[Owned component scopes](owned-component-scopes.md) now add process-local
`OwnedResourceScope` and state-sync owner barriers. [Owned IDB connections](owned-idb-connections.md)
add database owners and transaction leases. [Atomic outbox deltas](atomic-idb-outboxes.md)
now preserve updated peers' pending mutations; old/new-client lifetime fencing,
general host migration and durable execution/recovery remain pending. This gate grants no
runtime authority. The later [core Web host slice](core-web-plugin-host.md)
removes the application's dependency on the mixed Plugin runtime and adds typed
slots with owned inventory observations. Published SDK/native ABI retirement and
production security-policy acceptance remain pending.

[Provider UI authoring and ownership](provider-ui-ownership.md) adds the pure
`@cowboy/provider-authoring` contract and a separate core interactive-view owner.
Field/message links are checked at compile time; admission, permissions and
asynchronous lifetime remain core responsibilities, never an installable UI host.

## Plugin boundary

A plugin manifest has an exact ID, SemVer version, kind, entry point, and exact
component dependency list. The initial kinds are:

- `agent_provider`: an installable agent integration whose entry point is the
  existing signed, data-only Provider package contract;
- `authentication_provider`: a signed, data-only product-login integration.
  Payload schema 1 selects OIDC; schema 2 also selects local password and
  WebAuthn. The Controller owns credentials, account mappings, protocol
  execution, and Cowboy sessions; the package cannot execute provider code;
- `code_intelligence`: an isolated code-intelligence integration. The first is
  the separately built GPL Zed adapter and its exact private server. Schema 2
  owns both the executable graph and launch/readiness bindings; schema 1 is
  retained for previously published legacy adapters;
- `telemetry_backend`: a data-only exporter contract under the existing
  explicit Controller and Machine telemetry policy;
- `workspace_extension`: repository resource views, exact Plugin dependencies
  and a user-managed connection capability, rendered by the common workbench;
- `memory_backend`: a data-only memory service declaration (protocol, exact
  MCP tools, recall/observe) whose binding is Machine-private;
- `call_profile`: data-only managed-call purposes and agent-facing guidance;
  the call mechanism and sandbox remain core.

The generic Plugin identity is the repository, publication, discovery,
installation, rollback, and uninstall boundary. A Provider package is only the
typed payload of an `agent_provider` Plugin: it has no independent signature,
Catalog, installation slot, or Machine lifecycle. Zed uses the same signed
Plugin generation lifecycle while remaining process-isolated rather than an
in-process Rust dependency.

## Component boundary

Components contain reusable implementation or contracts, never an independently
installed product. Every active component is also a distributable package: Cargo
crates for Rust SDKs and npm source packages with explicit exports for
TypeScript, schemas, and runtime tooling. Cowboy consumes TypeScript components
by package name rather than reaching into their source directories. The registry
includes the plugin contract, versioned plugin host API, Web app shell, reactive
store, optimistic sync and IndexedDB adapter, Provider SDK/UI/runtime tooling,
and the Zed code-intelligence contract. Cowboy no longer stages or imports
`shared-utils`.

Each component release records:

- an exact component version;
- every source path owned by that component;
- a deterministic SHA-256 digest of those sources.
- its Cargo/npm package name and package manifest.

Plugins pin exact component versions and the exact component release. Ranges,
implicit workspace versions, private packages, missing public exports, and
moving references are invalid.

## Independent Plugin releases

Antigravity 0.2.19 changes only its host data: it declares the `percent`
desktop widget shape, so the Desktop top bar shows its per-group quotas (one
column per group, each at its binding window) without a component release.

Component release 3.134.0 records provider-runtime 1.3.18, whose
`cowboy-antigravity-acp` 1.0.16 hosts subagents in the proxy: the facade's
`Subagent` and `StopSubagent` tools start another native `agy` session inside
the calling session's turn and report it as an ACP subagent, so Antigravity
sessions get the same rows, stops and observed transcripts as Claude and Codex
([Subagents](antigravity-real-accounts-design.md#subagents)). Antigravity 0.2.18
pins it. The other six Providers pinning provider-runtime advance one patch
version (claude-code 3.20.64, claude-deepseek 3.1.81, codex 3.6.23,
codex-deepseek 3.1.81, deepseek-harness 1.0.39, grok 3.2.22).

Component release 3.133.0 records provider-runtime 1.3.17, whose
`cowboy-antigravity-acp` 1.0.15 asks Cloud Code's quota summary with `agy`'s
own client identity (`antigravity/cli/<version>`), which answers the Gemini
weekly and 5-hour groups and Pro's separate Argon quota
([Client identity decides the summary](antigravity-real-accounts-design.md)).
Antigravity 0.2.17 pins it. The other six Providers pinning provider-runtime
advance one patch version (claude-code 3.20.62, claude-deepseek 3.1.80,
codex 3.6.21, codex-deepseek 3.1.80, deepseek-harness 1.0.38, grok 3.2.20).

Component release 3.132.0 records provider-runtime 1.3.16, whose
`cowboy-antigravity-acp` 1.0.14 adds a "Gemini models" group to an
Antigravity account's quota report from Cloud Code's model catalogue
(`fetchAvailableModels`), because the quota summary covers only the Claude
and GPT models ([Gemini quota](antigravity-real-accounts-design.md)).
Antigravity 0.2.16 pins it. The other six Providers pinning provider-runtime
advance one patch version (claude-code 3.20.61, claude-deepseek 3.1.79,
codex 3.6.20, codex-deepseek 3.1.79, deepseek-harness 1.0.37, grok 3.2.19).

Component release 3.131.0 records provider-runtime 1.3.15, whose
`cowboy-antigravity-acp` 1.0.13 answers Cowboy's review schema through a
stated contract, an exact check and at most two nonce-bound corrections,
sending only a validated reply ([managed calls](managed-agent-calls.md)).
Antigravity 0.2.15 pins it and declares `--cowboy-output-contract=v1`; the
Controller admits explicit fresh Antigravity reviews and design reviews (with
that schema) and text analyses besides context asks. The other six Providers
pinning provider-runtime advance one patch version (claude-code 3.20.60,
claude-deepseek 3.1.78, codex 3.6.19, codex-deepseek 3.1.78,
deepseek-harness 1.0.36, grok 3.2.18).

Component release 3.129.0 records provider-runtime 1.3.14 with
`cowboy-antigravity-acp` 1.0.12 and `cowboy-antigravity-launch` 1.1.2: the
read-only managed profile for context asks. Antigravity 0.2.14 pins them and
declares `provider.managed-profiles.v1` with one `read_only_v1` profile (no
`image_v1`). Before it starts `agy` the proxy validates its execution
descriptor, the binding's managed record (exact profile and parent session)
and its agreement with the launch flag; it learns managed mode from that
binding and the target keeper's announcement. A managed child gets only the
facade's read tools and `RunCommand`, no Cowboy MCP, folder MCP, memory,
repository skills or slash commands, text-only prompts carrying their managed
round (no output schema) and exactly one native session, to which every
request, cancel, permission and update is bound. The `PreToolUse` hook
applies the ordinary policy only for the proxy's exact `ordinary` marker,
and nothing `agy` asks for is allowed before the runtime settles; the
launcher refuses every managed profile (see
[Managed read-only context asks](antigravity-provider-plan.md#managed-read-only-context-asks)).
The other six Providers pinning provider-runtime advance one patch version
(claude-code 3.20.59, claude-deepseek 3.1.77, codex 3.6.18, codex-deepseek
3.1.77, deepseek-harness 1.0.35, grok 3.2.17).

Component release 3.128.0 records provider-runtime 1.3.13, whose
`cowboy-antigravity-acp` 1.0.11 lists a repository's skills
(`.agents/skills`, `.claude/skills`) in an Antigravity session's projection
and offers each as a slash command, as Claude and Codex discover them
natively ([Repository skills](antigravity-real-accounts-design.md#slash-commands-available_commands_update)).
Antigravity 0.2.13 pins it and adds the Claude Opus 5.5 High and Claude
Sonnet 5.5 High cards that `agy` serves on the Google account. The other six
Providers pinning provider-runtime advance one patch version (claude-code
3.20.58, claude-deepseek 3.1.76, codex 3.6.17, codex-deepseek 3.1.76,
deepseek-harness 1.0.34, grok 3.2.15).

Component release 3.122.0 records provider-runtime 1.3.8, whose
`cowboy-antigravity-acp` 1.0.6 makes `agy`'s native `/compact` (the Web's
compact action) visible: the proxy reports it as ACP's typed
`compaction_update` and restates the session's Cowboy context (routing,
environment, repository and extension guidance) on the first owner prompt
after a completed compaction. Antigravity 0.2.7 pins it and tightens its mark
view box to `19.67 20 140 140`, matching the optical size of the Claude and
Codex marks. The other six Providers advance one unpublished patch version
(claude-code 3.20.53, claude-deepseek 3.1.71, codex 3.6.11, codex-deepseek
3.1.71, deepseek-harness 1.0.29, grok 3.2.10).

Component release 3.121.0 records Provider SDK 3.7.0 (plugin-sdk 1.20.0,
provider-ui 3.8.0, provider-authoring 1.0.19), which adds the levitating
activity indicator (`mark_levitate`, normalized to `asset_levitate`); a
package using it requires SDK 3.7.0, so older Controllers, Webs and Machines
skip it. Antigravity 0.2.6 uses it (a 2.6 s period), takes the official arch
mark, and gains the session extensions of a Claude or Codex session (Cowboy
MCP, MCP bindings, Matrix memory; design "Session extensions"): it declares
`provider.mcp-bindings.v1`, and provider-runtime 1.3.7 ships
`cowboy-antigravity-acp` 1.0.5 with `@cowboy/memory-client` 0.2.2 (an
injectable transport) as a component module of its archive. Every Plugin pins the
SDKs and advances one patch version: claude-code 3.20.52, claude-deepseek
3.1.70, codex 3.6.10, codex-deepseek 3.1.70, deepseek-harness 1.0.28, github
0.2.15, grok 3.2.9, machine-browser 1.0.12, matrix 1.0.13, review 1.0.16 and
zed 1.39.10.

Component release 3.120.0 records provider-runtime 1.3.6, whose
`cowboy-antigravity-acp` 1.0.4 shows the facade's tool calls as native ones
(Read, List, Search, Write, Edit, Run and Stop with their ACP kind, title,
target path and an edit's diff) and approves a facade tool's permission
request itself unless the session is in `agy`'s `plan` mode, so Antigravity
sessions do not prompt by default. Antigravity 0.2.5 pins it, takes a
gradient arch mark, a rising-particle activity glyph with its own phrases,
and new Recommended cards: Gemini 4 Neon · High (strongest), Gemini 4
Hydrogen · Extra and Gemini 4 Hydrogen · High (default). Neon gains one index
point from Extra over High (61 vs 60 on the Artificial Analysis Intelligence
Index) while Hydrogen gains three (54 vs 51) at half Neon's price, so Neon is
offered at High and Hydrogen at both.
The other six Providers again advance one unpublished patch version
(claude-code 3.20.51, claude-deepseek 3.1.69, codex 3.6.9, codex-deepseek
3.1.69, deepseek-harness 1.0.27, grok 3.2.8).

Component release 3.119.0 records provider-runtime 1.3.5, whose
`cowboy-antigravity-acp` 1.0.3 opens `agy`'s session in the target's
workspace: the worker names its runtime directory on the runtime Machine, as
for every Provider with an execution binding, and the proxy rewrites exactly
that path (`binding.runtime.cwd`) to the binding's canonical workspace root;
1.0.2 refused it as outside the workspace. Antigravity 0.2.4 pins it; the other
six Providers again advance one unpublished patch version (claude-code
3.20.50, claude-deepseek 3.1.68, codex 3.6.8, codex-deepseek 3.1.68,
deepseek-harness 1.0.26, grok 3.2.7).

Component release 3.118.0 records provider-runtime 1.3.4, whose
`cowboy-antigravity-acp` 1.0.2 accepts the execution descriptor with
systemd's `LoadCredential` mode (root 0440 plus an ACL for the unit's user):
1.0.1 refused any group bit, so the first OVH runtime stopped before
`initialize`. Group write and any access for others are still refused.
Antigravity 0.2.3 pins it. Every other Provider that pins provider-runtime
advances one patch version without behavior changes and stays unpublished
until a later release needs it: claude-code 3.20.49, claude-deepseek 3.1.67,
codex 3.6.7, codex-deepseek 3.1.67, deepseek-harness 1.0.25 and grok 3.2.6.

Component release 3.117.0 changes only Antigravity: 0.2.2 declares split
execution (`provider.execution-jsonrpc.v1`, the shared `jsonrpc_v1` executor
digest), so an Antigravity session runs `agy` on OVH with its workspace tools on
the development Machine, following the owner decision in
[Antigravity real accounts](antigravity-real-accounts-design.md#split-execution-declaration-owner-decision-2026-11-20).
Every other Plugin keeps its 3.116.0 version and closure.

Component release 3.116.0 (unpublished) records provider-runtime 1.3.3, whose
lock moves the Cowboy-owned `cowboy-antigravity-acp` to 1.0.1: `agy-proxy`
reads the plan `agy` reports in `_meta.antigravity.tier` of a session result
and carries it as `_meta.cowboy.account_plan` (`{id, name}`, never the tier's
`upgradeUri`) on every `usage_update`, which the Controller shows on the
account card; the Controller accepts it only beside the proxy's
`_meta.cowboy.sanitizer: 1` marker and from a session pinned to Antigravity
0.2.1 or later. A session result is forwarded before any reading behind it,
which already carries the plan; a prompt result still waits for its turn's
evidence, with the turn observed until then. The raw tier is
removed from every `_meta` of every forwarded message without altering any
other field. Antigravity 0.2.1 also adds Recommended cards for Gemini 3.8
Flash Medium and Low; Gemini 3.8 Flash High stays the default. Every other Provider
that pins provider-runtime advances one patch version without behavior
changes: claude-code 3.20.48, claude-deepseek 3.1.66, codex 3.6.6,
codex-deepseek 3.1.66, deepseek-harness 1.0.24 and grok 3.2.5. Native pins are
unchanged.

Component release 3.115.0 records provider-sdk 3.6.0, plugin-sdk 1.19.0,
provider-ui 3.7.0, provider-authoring 1.0.18 and provider-runtime 1.3.2.
Runtime contracts gain an optional `isolation` policy (SDK 3.6.0+):
`account_sandbox_v1` runs every process of a named account (runtime, login,
logout and probe) as that account's own system identity through the host's
`cowboy-account-sandbox` daemon, and requires `provider.native-custody.v1`
and the `ipv4_only_v1` network policy. Older hosts reject the unknown field;
a Machine without a reachable daemon reports the Provider unavailable.
Antigravity 0.2.0 declares it; its launcher 1.1.0 adds the `settings.json`
built-in tool denial list and runs from the daemon's runtime archive. The
shared `execution-connection.mjs` of provider-runtime 1.3.2 also connects to
`unix:` execution endpoints (the socket a sandboxed unit reaches its worker
through); `ws://` endpoints are unchanged, so the Claude and Codex runtimes
that bundle it keep their behavior. Its lock adds the Cowboy-owned `node_npm`
package `cowboy-antigravity-acp` 1.0.0 (a `cowboy:` source whose integrity
binds its own files and the shared modules it bundles; no primary registry
package): Antigravity's `agy-proxy` ACP proxy, tool facade and `PreToolUse`
hook, its second `provider_adapter` (`antigravity-proxy`), which the Machine
places below `proxy/` in the account-sandbox runtime archive. The launcher
drops its `settings.json` (the policy now ships with the proxy), and
Antigravity's `error_rules` classify `antigravity_backend_refusal:<class>`
turn errors.
The proxy also ends a turn in which a built-in tool ran with
`antigravity_policy_violation:<tool>` (detection behind the gate), which a
further error rule classifies. Every
other Plugin pins the SDKs and advances one patch version without behavior
changes: claude-code 3.20.47, claude-deepseek 3.1.65, codex 3.6.5,
codex-deepseek 3.1.65, deepseek-harness 1.0.23, github 0.2.14, grok 3.2.4,
machine-browser 1.0.11, matrix 1.0.12, review 1.0.15 and zed 1.39.9.

Component release 3.114.0 records provider-runtime 1.3.1, whose lock moves
native Codex (`openai-codex`) to 0.169.1 for both `codex` and
`codex-deepseek`. Every Provider that pins provider-runtime advances one patch
version: antigravity 0.1.1, claude-code 3.20.46, claude-deepseek 3.1.64,
codex 3.6.4, codex-deepseek 3.1.64, deepseek-harness 1.0.22 and grok 3.2.3.
Only the two Codex Providers change behavior; see
[the release record](releases/codex-0169-2026-11-18.md).

Component release 3.113.0 records provider-sdk 3.5.0, plugin-sdk 1.18.0,
provider-ui 3.6.0, provider-authoring 1.0.17 and provider-runtime 1.3.0, and
adds the `antigravity` Provider 0.1.0 (Linux x86_64 only). Runtime contracts
gain the `provider.native-custody.v1` capability (SDK 3.5.0+): every account is
a named, Machine-local native login and no credential enters the Controller
candidate, vault or replica pipeline. Readers that predate the fence reject the
unknown capability instead of treating the logins as portable. The runtime
lock gains two
recipe kinds: `vendor_archive`, a vendor-hosted native archive the Machine
downloads from the vendor's pinned URL and that publication never copies into
the Catalog, and `owned_archive`, a deterministic archive of a Cowboy-owned
launcher and data files. Every Provider that pins provider-runtime advances one
patch version without behavior changes: claude-code 3.20.45, claude-deepseek
3.1.63, codex 3.6.2, codex-deepseek 3.1.63, deepseek-harness 1.0.21, grok
3.2.2, github 0.2.13, machine-browser 1.0.9, matrix 1.0.11, review 1.0.14 and
zed 1.39.8.

Component release 3.112.0 records Zed 1.39.7, whose adapter dependency policy
records the chrono "unmaintained" notice RUSTSEC-2026-0543 (no vulnerability;
the pinned Zed crates require chrono). No component changes.

Component release 3.111.0 records provider-sdk 3.4.0, plugin-sdk 1.17.0,
provider-ui 3.5.0 and provider-authoring 1.0.16. Native identity gains the
`owner_declared_v1` recipe: the owner names the subscription when adding the
account, the Machine reads no login file, and the identity subject is the named
Cowboy account ID, which the Controller requires to match the account that
authorized the login. It cannot serve the default account. Runtime contracts
gain an optional `network` policy; `ipv4_only_v1` makes the Machine launch the
worker unit with `RestrictAddressFamilies=AF_INET AF_UNIX AF_NETLINK` and refuse
to launch it outside the systemd user spawn mode. Native login and logout
processes are not yet restricted. Both require Provider SDK 3.4.0, and older
hosts reject the unknown runtime field. Every Plugin pins the SDKs and advances
one patch version without behavior changes: claude-code 3.20.43,
claude-deepseek 3.1.62, codex 3.6.1, codex-deepseek 3.1.62, deepseek-harness
1.0.20, github 0.2.12, grok 3.2.1, machine-browser 1.0.8, matrix 1.0.10, review
1.0.13 and zed 1.39.6.

Component release 3.110.0 records provider-sdk 3.3.0, plugin-sdk 1.16.0,
provider-ui 3.4.0 and provider-authoring 1.0.15. Named subscription support is
derived from the signed manifest: a Provider supports named native accounts
exactly when an authentication method carries a native identity recipe.
Recipes gain an optional `subject_claim` (the subject is a claim of a compact
JWT the CLI wrote into the fresh login home) and the `*` path component (the
sole member of an object). Both require Provider SDK 3.3.0. Codex 3.6.0 adds the
native `chatgpt-subscription` method and Grok 3.2.0 adds `xai-subscription`;
their legacy portable credential fingerprints are unchanged. Every other Plugin
pins the SDKs and advances one patch version: claude-code 3.20.42,
claude-deepseek 3.1.61, codex-deepseek 3.1.61, deepseek-harness 1.0.19, github
0.2.11, machine-browser 1.0.6, matrix 1.0.9, review 1.0.12 and zed 1.39.5,
without behavior changes.

Component release 3.109.0 records provider-runtime 1.2.2: Claude Code 2.1.326
with Claude Agent ACP 0.107.3, Codex ACP 2.7.0 with native Codex 0.166.0, Grok
1.0.77 and dsh 0.2.2-rc.1. Every Plugin that pins provider-runtime advances one
patch version: claude-code 3.20.41, claude-deepseek 3.1.60, codex 3.5.30,
codex-deepseek 3.1.60, grok 3.1.60 and deepseek-harness 1.0.18. See
[the release record](releases/agent-runtimes-2026-11-17.md).

Component release 3.108.0 records plugin-sdk 1.15.0: the `mcp_server`
contract gains an optional MCP Apps declaration (signed inline views and
per-tool visibility, [`widget-apps-mcp.md`](widget-apps-mcp.md) §3), and
Machine preparation refuses a binding that grants an app-only tool to the
agent. Contracts without the declaration are unchanged. Every Plugin pins the
SDK and advances one patch version: claude-code 3.20.37, claude-deepseek
3.1.59, codex 3.5.29, codex-deepseek 3.1.59, deepseek-harness 1.0.17, github
0.2.10, grok 3.1.59, machine-browser 1.0.4, matrix 1.0.8, review 1.0.11 and
zed 1.39.3. No Plugin changes behavior.

Component release 3.107.0 records provider-runtime 1.2.1: Claude Code 2.1.321
(Claude Fable 5.2) with Claude Agent ACP 0.102.0 (Agent SDK 0.3.321). Every
Plugin that pins provider-runtime advances one patch version; only Claude Code
3.20.36 and Claude DeepSeek 3.1.58 change behavior. See
[the release record](releases/agent-runtimes-2026-11-10.md).

Component release 3.106.0 records Claude Code 3.20.35 and Claude DeepSeek
3.1.57, which keep the worker after the adapter's `incomplete_tool_call` turn
failure. No component changes.

Component release 3.105.0 records Codex 3.5.27, which declares
`provider.mcp-bindings.v1` and ships the MCP binding launch and execution CLI
probe runtime. No component changes.

Component release 3.104.0 lets Providers declare `provider.mcp-bindings.v1`:
provider-ui 3.3.1 accepts it in the Provider manifest validator, and
provider-authoring 1.0.14 follows. Claude Code 3.20.34 declares it, so its
sessions can receive an owner-granted signed MCP binding; the other Providers
advance one patch for the changed component. Native pins are unchanged.

Component release 3.102.0 adds the `machine_browser` Plugin kind (Plugin SDK
1.14.0, plugin-contract 1.13.0 with `machine-browser.schema.json`, release
schema 7, component kind `machine_browser_runtime`): a signed Linux browser
runtime whose contract names artifact-relative executables, the browser policy
binding and host requirements, while admission, isolation, streaming and the
viewer stay core (`machine-browser.md`). provider-ui 3.3.0 recognises the kind
and its release schema; provider-authoring 1.0.13 follows its exact peer pin.
First-party `machine-browser` 1.0.0 is declared through `plugin_additions`;
every existing Plugin advances one patch version for the new SDK pin, and the
Zed adapter moves with its Plugin. It follows scoped release 3.101.0, which
already recorded app-shell 1.1.41. Native pins are unchanged.

Component release 3.100.0 records the signed MCP server Plugin kind: plugin-contract
1.12.0 (`mcp_server`), plugin-sdk 1.13.0 (MCP contract, discovery and granted
source names), provider-sdk 3.2.0 (`provider.mcp-bindings.v1`), provider-ui 3.2.0
and provider-runtime 1.2.0 (execution CLI probe and MCP binding preparation);
provider-authoring 1.0.12 follows provider-ui. Every Plugin advances one patch
version for the changed contract and SDK; the Zed adapter moves with its Plugin.
Native pins are unchanged.

Scoped release 3.82.0 replaces the packaged plugin-api, app-shell and state-store
tests' external assertion imports with Node built-ins. It versions those packages,
state-store's dependent packages and the plugin-api consumers without changing
native pins. It removes the temporary root `@std/assert` dependency; package
closure validation remains strict and historical release records stay unchanged.

Scoped release 3.130.0 records app-shell 1.1.42 for the already-committed
reader top-edge change in `ab64737d0`, which 3.129.0 did not cover. No Plugin
closure or native pin changes; the worker registry input stays at 3.129.0.

Scoped release 3.101.0 records app-shell 1.1.41 for the already-committed
tool detail control change in `278c013ba`, which 3.100.0 did not cover. No
Plugin closure or native pin changes.

Release 3.98.0 records Zed Plugin 1.39.0 (agent-owned mirror reload) and
app-shell 1.1.40 for the already-committed floating-island changes in
`270f49cd9`, which 3.97.0 did not cover. The worker registry input follows the
new release; native pins are unchanged.

Scoped release 3.85.0 records app-shell 1.1.39 for the already-committed
floating dock, footer edge and test-assertion changes after 3.84.0
(`bd212af9`..`ca711c65`); no Plugin depends on that component, so the worker
registry input is unchanged. Native pins are unchanged.

Scoped release 3.81.0 records app-shell 1.1.36 for the already-committed
reader edge and dock changes (`b1234031`..`c9c226a3`); no Plugin depends on that
component, so the worker registry input stays at 3.79.0. The plugin-api and
state-store tests keep their recorded bytes (`@std/assert`), so neither package
nor its consumers change. Claude Code 3.20.28, Claude DeepSeek 3.1.52, Codex
3.5.22 and Grok 3.1.53 version the Biome reformatting of their sources
(`9bb295d9`) without a component release. Native pins are unchanged.

Scoped release 3.80.0 records app-shell 1.1.35 for the already-committed
sheet change `3f6fd69f` (no grab pills); no Plugin depends on that component,
so the worker registry input stays at 3.79.0. Native pins are unchanged.

Scoped release 3.78.0 records DeepSeek Harness 1.0.10, whose Recommended preset
uses dsh's advertised model option value. Native pins and its component release
3.77.0 are unchanged.

Component release 3.77.0 records provider-runtime 1.1.28 for
claude-deepseek-gateway 0.1.6. dsh cannot send custom headers, so its launcher
places the opaque session identity in the loopback gateway URLs and the gateway
restores it, attributing chat and fetch usage to the session. DeepSeek Harness
also gains a Recommended preset. Native pins are unchanged.

Component release 3.79.0 records matrix 1.0.5, which declares macOS: a data-only
memory backend runs wherever its Machine does.

Component release 3.76.0 records memory-client 0.2.1, which uses the session's
Machine relay and never fails a launch or holds a turn beyond its budget. Claude
Code and Codex advance one patch version. Native pins are unchanged.

Component release 3.75.0 records provider-runtime 1.1.27 for
claude-deepseek-gateway 0.1.5. The 0.1.4 gateway reported fetches under an
operation the Controller's usage table refuses, which stalled that producer's
usage queue; 0.1.5 uses the admitted dimensions. Every Plugin that pins
provider-runtime advances one patch version. Native pins are unchanged.

Component release 3.74.0 records provider-runtime 1.1.26. DeepSeek Harness now
selects DeepSeek's Fetch API for `web_fetch`, because dsh's local backend refuses
fake-IP DNS answers, and reaches it through claude-deepseek-gateway 0.1.4, which
keeps the key and reports each successful fetch. Every Plugin that pins
provider-runtime advances one patch version; only deepseek-harness and
claude-deepseek change behavior. Native pins are unchanged.

Scoped release 3.73.0 records app-shell 1.1.34 for the already-committed
loading-spinner changes `b2053d73` and `f7bd4bbd`; no Plugin depends on that
component. It also records Claude Code 3.20.22, whose usage collector badges
the former Max 20x plan as Ultra from the credential's rate-limit tier. Native
pins and its component release 3.72.0 are unchanged.

Component release 3.72.0 retires the Gemini Agent Provider, sunset on
2026-10-20. Its explicit `plugin_removals: ["gemini"]` migration drops
`gemini` from the active Plugin set and closure; every earlier release that
records it stays unchanged. Removing the Gemini CLI pin and package from the
shared runtime records provider-runtime 1.1.25, and replacing the Gemini
fixtures in the SDK's own tests records provider-sdk 3.1.17 with its exact
Cargo consumer plugin-sdk 1.12.4. provider-ui 3.1.23 accepts that SDK version
for package validation, and provider-authoring 1.0.11 follows its exact peer
pin. Every remaining Plugin pins plugin-sdk, so
all ten advance one patch version without changing their upstream executable
pins; the Zed adapter package follows its Plugin identity. Because a package
requires a Machine whose Plugin and Provider SDK are at least its pins, a
Machine needs a host built from this release before it accepts these Plugin
versions. Historical sessions, usage and Catalog entries naming `gemini`
resolve through generic package-less behavior.

Candidate release 3.60.0 records provider-sdk 3.1.15 for optional signed native
identity recipes, provider-ui 3.1.21 for exact authentication Machine discovery,
and their transitive package consumers plugin-sdk 1.12.2 and provider-authoring
1.0.9. All ten consuming Plugins advance independently by one patch version;
their upstream executable pins remain unchanged. The Zed adapter package follows
its Plugin identity. Historical records are unchanged. This source registration
does not publish artifacts, activate Machines or open named-account admission.

Scoped release 3.54.0 records app-shell 1.1.28 for UI sources already present on
main but missing from the 3.53.0 digest. It changes no UI behavior. Review 1.0.1
shortens its optional review guidance; Codex 3.4.8 and Claude Code 3.19.37 update
the tests of that delivered guidance while retaining their runtime pins and
production adapter code. All historical releases and unrelated Plugin versions
remain unchanged. Source validation does not establish publication or activation.

Component release 3.49.0 records provider-runtime 1.1.14: Claude Code 2.1.296,
Claude Agent ACP 0.89.1, Codex CLI 0.160.1, Codex ACP 2.2.2, Gemini CLI 0.63.0
and Grok 1.0.50. Node and both DeepSeek gateways keep their exact pins. All six
Agent Plugins pin that shared runtime component and advance one patch version
each. Codex ACP 2.2 declares native Codex `^0.160.1`, so Codex 0.161 and 0.162
are not adopted. Claude Code 3.19.20 also follows what native 2.1.296 changed
for the target tools it answers; see
[`releases/agent-runtimes-2026-10-11.md`](releases/agent-runtimes-2026-10-11.md).
Zed and GitHub are unaffected.

Component release 3.46.0 changes no component. It records Claude Code 3.19.18,
Codex 3.4.2, Claude DeepSeek 3.1.33 and Grok 3.1.34, whose signed collectors
now call the bundled Bun runtime directly instead of a host-provided
compatibility global. A Machine host from this release on no longer preloads
that global, so these are the oldest collector releases it can run; converge a
Machine's Plugins before replacing its host.

Component release 3.45.0 moves the last build scripts and tests inside
component and Plugin sources to Bun. It records plugin-api 1.1.2,
app-shell 1.1.22, state-store 2.1.1, provider-ui 3.1.18 and provider-runtime
1.1.13 for those sources, and state-sync 1.5.1, state-sync-idb 1.8.1 and
provider-authoring 1.0.6 for their exact peer pins. The six Agent Plugins pin
the new plugin-api, provider-ui and provider-runtime and advance one patch
version each; Zed advances to 1.20.6 for its own runtime build script and keeps
its component pins. GitHub is unaffected. No component export, Plugin payload
contract or collector changed: these are build-time and test-time sources, and
the release exists because every source file is part of a recorded digest.

Scoped release 3.38.0 records app-shell 1.1.19 for the already-committed native
installed-bundle version comparison (`eebf0a5c`). No component or Plugin depends
on app-shell, so all other versions, Plugin pins and historical release records
remain unchanged. This registry repair allows the complete gate to validate
fresh main; it is not a Web, native-device or Plugin activation receipt.
The derived worker-registry input retains 3.37.0 for this app-shell-only tail;
the subsequent real Claude release 3.39.0 advances it normally.
Both the root gate and immutable worker build verify that derivation. After a
registry edit, regenerate it from the repository root in the pinned shell with
`nix develop -c bun tools/worker-registry-input.ts --write`.
SDK, Plugin and consumed component changes remain release-causing worker inputs;
the derived file is not an administrator-selected generation pin.

The component registry records the minimum Plugin version tested when a shared
component release is cut. A Plugin may subsequently increase its own version
without creating another component release or changing any sibling Plugin. When
a component release changes, schema 3 versions the affected dependency closure.
An unaffected Plugin keeps its exact source, version and historical
component-release pin; the active matrix is not a new identity for that Plugin.

Claude Code 3.19.38 independently fixes target Git context when the runtime
directory is not a repository. The target snapshot is supplied once through
native `prompt.context`; any runtime Git section is removed from
`session_context`. Native CLI 2.1.296, ACP 0.89.1 and all shared component pins
remain unchanged. This is a runtime change requiring packaged acceptance,
unlike the preceding 3.19.37 source-test update.

Claude Code 3.20.2 independently lengthens an idle stdio MCP server's
notification wait from 25 s to 300 s; a write still re-reads after 25 s
without a notification. Native pins and component release 3.60.0 are
unchanged. It is a runtime change requiring packaged acceptance.

Claude Code 3.20.3 runs every launcher Node process (ACP adapter, private
CLI, MCP relays, hook proxy, task waiter) with `--optimize-for-size`, which
about halves their resident V8 heap. Native pins and component release
3.60.0 are unchanged. It is a runtime change requiring packaged acceptance.

Claude Code 3.20.4 relays each target stdio MCP server through
`bin/cowboy-mcp-relay` (`plugins/claude-code/relay/`), a static Linux
executable speaking the Node relay's bridge protocol, instead of one Node
process per server. A target without it (macOS) keeps `mcp-proxy.mjs`. Native
pins and component release 3.60.0 are unchanged. It is a runtime change
requiring packaged acceptance.

Claude Code 3.20.5 also runs the waiter behind a native background task that
stands for a target command (`cowboy-mcp-relay task-wait <job>`) natively,
keeping `task-wait.mjs` where the binary is absent. Unlike the Node waiter, it
counts a refused bridge reply as a failure instead of asking again at once.
Native pins and component release 3.60.0 are unchanged. It is a runtime change
requiring packaged acceptance.

Claude Code 3.20.7 preloads the host's jemalloc into the ACP adapter and the
private CLI, the two Node processes every session keeps resident, where the
host ships it (`libjemalloc.so.2` under `/usr/lib/<arch>-linux-gnu`, as on
Ubuntu) and neither `LD_PRELOAD` nor `MALLOC_CONF` is already set. One arena
and a one-second dirty decay return freed native memory instead of keeping
glibc's fragmented peak: with the real adapter modules under a transcript-like
stream on OVH, 45 MiB became 34 MiB of anonymous memory for 6% more CPU. The
launcher removes the preload before it starts anything, so native Claude,
hooks and MCP servers keep their usual environment; hosts without the library
run as before. Native pins and component release 3.61.0 are unchanged. It is a
runtime change requiring packaged acceptance.

Claude Code 3.20.8 starts native Claude with `BUN_GC_TIMER_INTERVAL=100` unless
the environment already sets it. Bun then checks whether to collect every
100 ms: against a scripted API on OVH, 40 turns peaked at 147 instead of
182 MiB of anonymous memory and settled at 109 instead of 122 MiB, with no
measurable CPU cost while busy or idle. Native pins and component release
3.61.0 are unchanged. It is a runtime change requiring packaged acceptance.
Version 3.20.8 was never published: the change first shipped in 3.20.9 with
claude-agent-acp 0.98.0 (see the 2026-10-28 runtime release record).

Claude Code 3.20.10 clamps a Bash `timeout` above the maximum (10 minutes in
the foreground, 2 hours in the background) as native does, instead of refusing
the call with `Invalid tool limit`. Native pins and its component release
3.62.0 are unchanged. It is a runtime change requiring packaged acceptance.
Scoped release 3.63.0 records app-shell 1.1.33 for the already-committed
`9daa4cdb`; no Plugin depends on that component.

`cowboy-plugin-pack build` reads the Plugin manifest's `component_release`; it
does not read Cowboy's component registry or assume a Cowboy checkout as its
working directory. `just plugin-isolation-check <id>` copies the Plugin source
into an unrelated temporary directory and builds there. `provider-check`
exercises every first-party Plugin. This is the acceptance proof for moving
a Plugin to its own repository and release pipeline.

External publishers can use `nix build .#cowboy-plugin-pack` to obtain a
closure-pinned SDK CLI, including its signature tool. The `version` command
reports the exact SDK version; no Cowboy working directory is needed to build an
external package. Cardea's repository owns its own release recipes using this
tool, rather than copying the SDK or relying on an ambient compiler.

## Dependency-closure release rule

Release 3.60.2 records state-sync-idb 1.8.2 for the already-committed source
whose digest differed from the 1.8.1 record. No Plugin depends on that package;
this metadata correction changes no Plugin pins or runtime behavior.

The registry is one ordered history stored in two files. `components/registry.json`
keeps the active release and every release an in-tree `plugin.json` or the
worker pin (`components/worker-registry-input.json`) still references; every
other release moves unchanged, in the same schema, to
`components/registry-archive.json`. `src/plugin.rs`,
`tools/check-plugin-components.ts`, `tools/worker-registry-input.ts` and the
closure tests read both through `tools/component-registry.ts` (Rust:
`merged_component_registry`), which sorts by release version and refuses a
duplicate or a schema mismatch, so ordered validation, identity history and
installed Plugins pinned to an archived release behave as before. The merged
two-space encoding is byte-identical to the unsplit file, so the worker
registry digest and the worker generation do not change when a release is
archived; the archive is excluded from the per-file worker hash. Archive a
release only after no in-tree Plugin or worker pin references it.

`components/registry.json` remains append-only. Schema-2 entries (through 2.9.0)
still enforce strictly increasing versions for every Plugin between adjacent
releases. Schema 3 appends a separate, unchanged 3.0.0 migration baseline before
the first dependency-scoped change in 3.1.0. It never reinterprets old entries.

After that baseline, adding component identities requires an explicit nonempty
`component_additions` list on the new schema-3 release. It must equal exactly the
new IDs: undeclared additions, removals, duplicate/reused identities, missing
nodes and cycles fail. New dependency edges still version every affected
existing component and Plugin. Release 3.4.0 uses this additive migration for
`cowboy.provider-authoring` only; historical records and all seven Plugin pins
remain unchanged. This build-metadata migration is not runtime authority.

Each new release records all internal component-package edges and each Plugin's
exact source digest, component pins and component-release label. Plugin source
digests use the Git source list (including new non-ignored source before
commit), excluding local Cargo/Node build caches. Isolated builds use that same
list and compare package/host bytes with the in-tree SDK build, so an ignored
input cannot silently affect a published package. npm dependency, peer, optional
and development pins are checked separately; Cargo edges come from offline,
locked `cargo metadata`. Every declared internal edge is currently
conservatively release-causing, including contract/build inputs. Fine-grained
contract compatibility exemptions are not implemented. The matrix itself is
tested compatibility metadata, not an implicit dependency on every component.

A changed component requires a higher version, as do its transitive package
consumers. Missing nodes, cycles, ranges, stale pins, incomplete package source
coverage and symlinked release sources fail the gate. A Plugin may retain an old
component-release label only if its whole declared transitive closure has
identical component versions, digests, source ownership and package identities
in that release and the active matrix. A changed Plugin source snapshot or
binding requires a higher Plugin version. Adding/removing component or Plugin
identities still requires an explicit migration, not an inferred exemption.

Scoped release 3.36.0 records app-shell 1.1.18 for the already-committed update
countdown and uniform thin download indicator (`c601ecdb`, `6303fa48`). Those
changes had not updated the component source record. No Plugin depends on this
component, so all Plugin pins and minimum tested versions remain unchanged.
Claude 3.3.0 is an independent Plugin release against its unchanged 3.35.0 pin.

Scoped release 3.33.0 records provider-runtime 1.1.9: Claude Code 2.1.286,
Codex CLI 0.159.3, Codex ACP 2.1.0 and Grok 1.0.46. All six Agent Plugins
advance independently because they pin that shared runtime component; Gemini
CLI, Claude ACP, Node and both DeepSeek gateways retain their exact pins.
Codex recommends Sol 6.1 Medium/Max and defaults new sessions to Medium;
Claude's Sonnet card describes 5.5. Existing sessions retain their selections.
This matrix also records app-shell 1.1.17 for the already-committed keyboard
footer fix in `532119dd`, whose component version/digest had not been recorded.
No additional app-shell behavior changes in this release.

Scoped release 3.31.0 records provider-runtime 1.1.8 and Node 24.21.0 LTS.
The six Agent Plugins take independent patch versions for Claude Code 2.1.285,
Claude Agent ACP 0.84.0, Codex 0.159.2, Codex ACP 2.0.1, Gemini CLI 0.62.0,
and Grok 1.0.44. The exact DeepSeek gateway revisions and authentication
contracts remain unchanged. The Codex source patch retains bounded strict
UTF-8 framing, configured executable arguments and pending-request cleanup
while accepting the upstream valid unterminated final frame. GitHub keeps its
source, version and 3.30.0 component-release pin. Zed advances independently to
1.20.2 for the Rust lint-compatible fixed-size chunk iterator, retaining its
3.30.0 component pin and private upstream revision. Publication and Machine
upgrade acceptance are recorded separately from this source matrix.

Scoped release 3.29.0 records provider-runtime 1.1.7. Both DeepSeek gateways
advance to Columbus `27352e34`: `codex-deepseek` 0.3.0 serves `deepseek-flash`
(DeepSeek-V4.1-Flash, with image input), its retained `deepseek-v4-flash` alias
and `deepseek-v4-pro` through DeepSeek's native Responses API and deletes the
temporary Pro Chat adapter; `claude-deepseek` 0.1.1 only changes content-free
telemetry. Both DeepSeek Plugins move their presets, defaults and runtime model
names to DeepSeek's recommended `deepseek-flash`. Every other pin is unchanged,
but the shared lock is part of every Agent Plugin's closure, so all six take the
new closure and independent patch versions; Zed again retains its unchanged
source, version, and historical component-release pin.

Scoped release 3.28.0 records provider-runtime 1.1.6. Codex advances from 0.154.0
to 0.156.1 and Codex ACP from 1.11.0 to 1.13.1, including the adapter's exact ACP
SDK 1.5.0 lock; Grok advances from 1.0.30 to 1.0.41. The Codex CLI release adds
GPT-6 Sol and Luna to its model catalog, and every pinned preset model
(`gpt-5.6-sol`, `gpt-5.6-luna`, `gpt-6-astra`) still resolves, so the presets
stay as published. The one Codex runtime lock entry per component id is shared, so
`codex-deepseek` takes the same pins as `codex`; all six Agent Plugins take the
new closure and independent patch versions, and Zed again retains its unchanged
source, version, and historical component-release pin.

Scoped release 3.27.0 records provider-runtime 1.1.5. Claude Code advances from
2.1.278 to 2.1.280 and Claude Agent ACP from 0.79.0 to 0.81.0, including the
adapter's exact Agent SDK 0.3.280 lock. The CLI release adds Claude Opus 5.5
(`claude-opus-5-5`), so the Agent model list gains it without a Cowboy contract
change. Both Anthropic Providers share the one runtime lock entry per component
id, so `claude-deepseek` moves with `claude-code`; all six Agent Plugins take the
new closure and independent patch versions, and Zed again retains its unchanged
source, version, and historical component-release pin.

Scoped release 3.26.0 records provider-runtime 1.1.4. Claude Code advances from
2.1.272 to 2.1.278 and Claude Agent ACP from 0.77.0 to 0.79.0, including the
adapter's exact Agent SDK 0.3.274 lock. All six Agent Plugins move to the new
runtime component closure and receive independent patch versions; Zed retains
its unchanged source, version, and historical component-release pin.

The first scoped release changes only `state-store` (2.0.0), `state-sync`
(1.3.0) and `state-sync-idb` (1.3.0). All seven Plugin sources and their exact
2.9.0 pins are unchanged. Their signed packages, private runtimes, Catalog
formats and Machine installations are untouched. The Controller understands both
registry schemas and projects each Plugin's own release label; the registry is
embedded build metadata, so an old Controller/rollback binary still reads its
own matrix and the same signed Catalog. No new Catalog envelope or reader bridge
is needed.

Scoped release 3.3.0 adds state-sync-idb 1.5.0's explicit database owner, bounded
open results, transaction-terminal leases and exact state-store 2.1.0 peer.
The seven Plugin releases and their 2.9.0 pins remain unchanged.

Scoped release 3.5.0 adds state-sync 1.5.0's exact load-result handoff and
state-sync-idb 1.6.0's atomic outbox deltas. Database schema/record bytes and
all seven Plugin sources/pins remain unchanged; old blind writers still require
an upgrade. This core-only slice needs a Web release, not a Catalog transaction.

Scoped release 3.6.0 adds state-sync-idb 1.7.0's exact schema floor, optional
transaction-lifetime handles and strict bounded inspection. The core
[product dataset integration](product-sync-datasets.md) binds immutable
Service/user identity to Web persistence and authenticated socket negotiation;
its Controller/Web migration and recovery acceptance are independent of the
unchanged seven Plugin sources, pins and signed Catalog.

Scoped release 3.2.0 adds state-store 2.1.0's owned resource scope, state-sync
1.4.0's lifetime/barrier fixes, and the state-sync-idb 1.4.0 transitive peer
update. The same seven Plugin releases and exact 2.9.0 pins remain unchanged;
the [local lifecycle slice](owned-component-scopes.md) requires only a Web
activation, not a Catalog write or Machine installation.

Scoped release 3.10.0 records app-shell 1.1.2's Android system-haptic bridge
after integrating the native-shell change. No component or Plugin depends on
app-shell, so the release changes only that component; existing Plugin source,
versions and component-release pins remain unchanged. The matrix snapshots the
current independently versioned Plugins without rewriting historical entries.
This metadata repair does not publish a Catalog or activate a native shell.

Scoped release 3.12.0 records app-shell 1.1.3's already-integrated connection
banner change. App-shell has no component or Plugin consumers in the closure,
so only its package version/digest changes. The current Plugin sources are
snapshotted, including the independently versioned Zed 1.18.0 candidate; their
historical component pins and existing signed releases remain unchanged.

Scoped release 3.25.0 records app-shell 1.1.16: the update bar shows its press
as a control. A full-width tinted bar at the top of a screen already means
"notice", so the imperative sentence inside it did not read as a button; the
verb now leaves the sentence and is drawn as an outlined `UpdateActionPill`
naming what the next press does. The pill is decoration — `pointer-events:
none` and `aria-hidden` — so the target stays screen-wide and a screen reader
still hears one control. Only app-shell's version/digest changes; it still has
no component or Plugin consumers, and no Plugin manifest, signed bytes or
component pin changes. No Catalog publication or Machine upgrade is needed.

Scoped release 3.24.0 records app-shell 1.1.15: a download nobody asked for is
a translucent hairline at the top edge rather than a bar of text the user can
only watch, and the bar arrives with the thing it announces
(`updateShowsHairline`, `updateHairlineSx`). `UpdatePhase` gains `rejected`
and `abandoned` for a build this device watched fail to start: the worker
serves the previous generation again, stops promoting that deploy in a
version-scoped state cache, and the bar carries a warning-toned notice whose
press lifts the rejection. `useAutoUpdate` gains `beforeReload` so the app can
record the swap its next boot has to sign for. Only app-shell's version/digest
changes; it still has no component or Plugin consumers, and no Plugin manifest,
signed bytes or component pin changes. No Catalog publication or Machine
upgrade is needed.

Scoped release 3.23.0 records app-shell 1.1.14: the shell-refresh progress
added in 3.22.0 is opt-in. That reply port's other caller is the previous
build's client, which resolves on the first message it receives and reads one
without `ok` as a failed download, so an unasked progress message stranded
every open older page on "could not be downloaded yet · retrying" against a
download that had in fact succeeded. The worker now streams batches only to a
client that sends `progress: true`, and an unflagged request keeps its exact
single-reply shape. Only app-shell's version/digest changes; it still has no
component or Plugin consumers, and no Plugin manifest, signed bytes or
component pin changes. No Catalog publication or Machine upgrade is needed.

Scoped release 3.22.0 records app-shell 1.1.13: the update banner is now the
update control. The deployed build is downloaded as soon as a deploy is
detected and the service worker streams its boot-asset count back, so the bar
fills with a real download and a press swaps builds from cache alone
(`downloadUpdate`/`reloadIntoUpdate` replace `applyUpdate`; `useAutoUpdate`
gains `phase`, `progress` and `requestUpdate`, and `update-presentation.ts`
owns the fill). The automatic countdown is unchanged and still arrives for a
user who never presses. Only app-shell's version/digest changes; it still has no
component or Plugin consumers, and no Plugin manifest, signed bytes or
component pin changes. No Catalog publication or Machine upgrade is needed.

Scoped release 3.21.0 records app-shell 1.1.12: a zoomed preview figure moves
like a scroll view instead of a dragged box. A released pan now coasts its
smoothed speed out on one compositor transition whose curve leaves at exactly
the speed the finger did, an axis the figure already fits is rigid rather than
elastic — a wide Mermaid diagram no longer drifts vertically while it is panned
sideways — a settle keeps its pan layer promoted instead of demoting it on the
frame the animation starts, and a finger landing mid-settle freezes the figure
where it is painted rather than jumping to the target. The same freeze fixes a
second zoom step taken mid-animation, which used to measure a stale rect and
bake a size the figure never had. Only app-shell's version/digest changes; it
still has no component or Plugin consumers, and no Plugin manifest, signed bytes
or component pin changes. No Catalog publication or Machine upgrade is needed.

Scoped release 3.20.0 records app-shell 1.1.11: a deployed build is applied by
the client itself on every surface. `useAutoUpdate` and the pure
`update-policy` rules move the countdown out of the desktop banner so the phone
runs the same policy, add the foreground dwell a resumed PWA needs, and re-arm
the one-second check that a held countdown previously dropped — a busy page
could stay on the old build until it was reloaded by hand. Only app-shell's
version/digest changes; it still has no component or Plugin consumers, and no
Plugin manifest, signed bytes or component pin changes. No Catalog publication
or Machine upgrade is needed.

Scoped release 3.19.0 records app-shell 1.1.10: a fullscreen preview also takes
the document background and colour scheme to its backdrop while it is open. The
iOS standalone status bar sits above the web view and a light iPhone PWA fills
that strip from the document, not from `theme-color`, so the bar stayed white
over a near-black preview; all three writes are snapshotted and restored on
close. Only app-shell's version/digest changes; it still has no component or
Plugin consumers, and no Plugin manifest, signed bytes or component pin
changes. No Catalog publication or Machine upgrade is needed.

Scoped release 3.18.0 records app-shell 1.1.9: the lightbox commits the neutral
paint when it swaps the pan layer's layout size. A CSS transition starts from
the previous style recalculation, so baking (or unbaking) the layout and
starting an animated transform in the same task interpolated the old transform
against the new layout — a 3x figure flashed to 9x and eased back over the
settle, the twitch at the end of a pinch. Only app-shell's version/digest
changes; it still has no component or Plugin consumers, and no Plugin manifest,
signed bytes or component pin changes. No Catalog publication or Machine
upgrade is needed.

Scoped release 3.17.0 records app-shell 1.1.8: the lightbox's baked pan layer
scales the plate's padding with it, so settling a pinch no longer widens the
content box and twitches the artwork, and a fullscreen preview takes the iOS
standalone status bar to its backdrop colour (through the sheets' own
`setStatusBarColor`, now shared) and restores the app's colour on close. Only
app-shell's version/digest changes; it still has no component or Plugin
consumers, and no Plugin manifest, signed bytes or component pin changes. No
Catalog publication or Machine upgrade is needed.

Scoped release 3.16.0 records app-shell 1.1.7: the lightbox writes its plate
onto an inline-SVG figure's own element. A host renderer paints a background on
the SVG root, and that inline style beat the component's class rule, so a light
figure lost its white plate and its dark strokes disappeared into the near-black
backdrop. Only app-shell's version/digest changes; it still has no component or
Plugin consumers, and no Plugin manifest, signed bytes or component pin changes.
No Catalog publication or Machine upgrade is needed.

Scoped release 3.15.0 records app-shell 1.1.6: a zoomed lightbox figure keeps
`flexShrink: 0`, so the baked pan layer is no longer shrunk back to the viewport
by the centring flex row — an inline SVG diagram stayed pinned at fit size
because its automatic flex minimum is zero. Only app-shell's version/digest
changes; it still has no component or Plugin consumers, and no Plugin manifest,
signed bytes or component pin changes. No Catalog publication or Machine
upgrade is needed.

Scoped release 3.14.0 records app-shell 1.1.5: the connection banner's update
action now downloads the deployed shell through the service worker before it
reloads and reports whether that finished (`applyUpdate(): Promise<boolean>`),
so a weak connection keeps the running build. Only app-shell's version/digest
changes; it still has no component or Plugin consumers, and no Plugin manifest,
signed bytes or component pin changes. No Catalog publication or Machine
upgrade is needed.

Scoped release 3.13.0 records app-shell 1.1.4's already-integrated gallery image
contract. Only app-shell's version/digest changes; it still has no component or
Plugin consumers. Current independently released Claude Code 3.1.28 and Zed
1.20.0 sources are snapshotted without changing their manifests, signed bytes
or 3.11.0 component pins. No Catalog publication or Machine upgrade is needed.

Run:

```sh
just plugin-check
just plugin-build <plugin-id>
```

`plugin-check` validates publishable package manifests and exports, source
digests, exact dependency pins, plugin and entry-point identity, Provider
payload versions, the Zed adapter version, legacy coordinated history, and new
dependency-closure transitions. `provider-check` additionally proves the Plugins
can build from isolated source copies. The repository-wide `just check` includes
both gates.

For release preparation, `--print-digests` and `--print-closure` print candidate
metadata only; they are not validation or publication commands. Normal
`bun tools/check-plugin-components.ts` validates
the complete tree in the pinned shell (Cargo metadata inherits that shell).

An explicit `plugin_additions` list permits new first-party Plugin identities in
a post-baseline closure release. It must exactly account for the additional
nodes; identity reuse, duplicates and undeclared additions are refused.
An explicit `plugin_removals` list likewise retires existing identities: each
must be present in the previous release and absent from the new one, and a
retired identity can never be added back. Undeclared removals are refused.
Existing release snapshots and component-change version rules remain immutable.

## Owned preference state

`@cowboy/state-store/core` has no React dependency. v2 requires explicit typed
serialize/deserialize functions and prevents a decoder from widening the store's
inferred type. Existing Web codecs preserve preference keys and encodings.
Listeners belong to subscriptions, with release on last unsubscribe; owned
instances additionally expose idempotent `dispose()`. Delayed callbacks are
fenced across listener/instance lifetimes. Failed storage access cannot leave a
committed value invisible to its subscribers. See the
[component contract](../components/state-store/README.md) for fallback
semantics. View cleanup neither deletes preferences nor cancels Machine work.

`just example-auth-build-all` discovers every login host under
`examples/authentication/` and requires its own manifest and SDK-only package
build. The examples include release-ready Password and Passkey source packages;
they are not additional Machine installations or an alternative release format.
They join `provider-check` so a shared SDK change exercises the local-login
migration prerequisites as well as all Machine Plugins.

## Layout

```text
components/
  registry.json
  plugin-contract/
  plugin-sdk/
  plugin-api/
  app-shell/
  state-store/
  state-sync/
  state-sync-idb/
  provider-sdk/
  provider-ui/
  provider-runtime/
  code-intelligence/
plugins/
  <agent-provider>/plugin.json + provider.json
  zed/plugin.json + adapter/
  github/plugin.json + contract.json
examples/authentication/
  <authentication-provider>/plugin.json + authentication.json
```

Machine Plugin state lives under `plugins/`. Startup atomically adopts the old
`providers/` root only when the new root does not yet exist. Provider-named auth
replica paths remain capability-specific state, not an extension lifecycle.
