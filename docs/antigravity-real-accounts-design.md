# Antigravity real accounts on OVH — design

Status: design, revision 1, 2026-11-18. Nothing here is implemented. It extends
[`antigravity-provider-plan.md`](antigravity-provider-plan.md) Phases 3 and 4
under the
[Antigravity consumer-subscription boundary](requirements.md#antigravity-consumer-subscription-boundary).

## Owner decisions (2026-11-18)

- Four real Google accounts, each a named Cowboy account the owner names
  (for example an animal), each signed in by the owner personally.
- Runtime and sign-in on **OVH**, like named Claude accounts: the sign-in page
  opens only in that account's isolated Machine browser on OVH (the
  2026-11-17 rule in [Machine browser](machine-browser.md)), and `agy` runs on
  OVH while workspace tools execute on the session's development Machine.
- The dedicated free-tier test account goes through the whole flow on OVH
  first; real accounts are then admitted **one at a time**, each after a soak
  of the previous one without verification or enforcement signals.
- All accounts share OVH's IPv4 egress (no per-account exit). Linkage by
  address is accepted; usage pacing is the mitigation.

## Threat model

| Threat | Layer |
|---|---|
| One account's credential read, used or overwritten by another account's `agy` (prompt injection into `run_command`, a hostile repository, an `agy` bug) | L1 custody, L2 confinement |
| Any Antigravity process reads another Provider's credentials, or another Provider's agent reads an Antigravity token | L2 confinement (both directions) |
| Google links the four accounts through a shared browser, keyring item or installation id | L1, L4 |
| Google sees sign-in and use from different locations, or OVH's IPv6 (`/sorry`, `UNSUPPORTED_LOCATION`) | L3 network |
| Unattended or fan-out use, parallel turns, retry storms, quota-driven switching | L5 account controls |
| A suspended or verification-blocked account keeps being used | L5 |
| Credentials leave OVH | L1 (done) |

Out of scope: per-account egress addresses; hiding that the accounts belong to
one person; protection against the trusted Machine supervisor itself (same
trust boundary as today, `requirements.md`).

## L1 Custody (done in 3.113.0)

Native-only and named-only (`components/wire/src/provider_account.rs`), signed
`provider.native-custody.v1`, per-account per-generation native home, token as
a 0600 file under it, impossible D-Bus address so no shared keyring item, no
default account, `installation_id` per home.

## L2 Filesystem and process confinement — Landlock

The OVH (Ubuntu, `kernel.apparmor_restrict_unprivileged_userns=1`) and Hawk
(NixOS) kernels both list `landlock` in `/sys/kernel/security/lsm`. Landlock
needs no privilege and no user or mount namespace, is inherited by every
descendant and cannot be removed by them. systemd `InaccessiblePaths=` and
`TemporaryFileSystem=` need a mount namespace, which the OVH user manager
cannot create, so they are not used.

**Signed declaration.** Runtime contracts gain an optional
`filesystem: "account_home_v1"` policy (Provider SDK floor raised with it;
older hosts reject the unknown field). Antigravity declares it.

**Enforcement point.** The Machine applies the ruleset in `pre_exec` of every
process it starts for that Provider: the worker's agent subprocess (`src/acp.rs`
spawn), the native login and logout (`authentication_process`), and the
install probes. The host refuses to launch when the kernel's Landlock ABI is
below the minimum (6: filesystem rights including truncate and ioctl on
devices, plus signal and abstract-Unix-socket scoping), or when the ruleset
cannot be fully enforced. No fallback.

**Rules (`account_home_v1`).** Everything not listed is denied for all handled
filesystem rights:

| Path | Access |
|---|---|
| this account's current native generation `home` | read-write |
| this Provider generation's installed `content` | read and execute |
| `/nix/store`, `/usr`, `/lib*`, `/bin`, `/etc` (read-only system) | read and execute |
| `/dev/null`, `/dev/urandom`, `/dev/tty`, `/proc/self` | as needed |
| a private per-worker temporary directory (`TMPDIR`) | read-write |
| local mode only: the session cwd's workspace root | read-write |
| split mode: the empty same-path mirror cwd on OVH | read-only |

Not listed, therefore denied: every other account's directory, other
Providers' state under the Machine state root, `~/.ssh`, `~/.gemini`,
`~/.claude*`, `~/.codex`, `~/.grok`, the worker sockets of other sessions.
Scoping denies signals to, and abstract-socket connections with, processes
outside the domain. In split mode `agy` has no workspace rights at all: every
file and command goes through the facade (below) to the target, whose own
execution host enforces that session's binding.

**Reverse direction.** Landlock is allow-list only: it cannot deny one
directory to an otherwise unrestricted process. Protecting Antigravity tokens
from other Providers' agents therefore means confining those agents too, with
`account_home_v1` or a variant that grants their workspace and their own
account home. Until that lands, this design does not claim that a Claude or
Codex session on OVH cannot read an Antigravity token. *(Open question 1.)*

**Acceptance.** A canary file in account A's home and in another Provider's
state: from account B's `agy` session, `run_command cat <path>` and the
facade both fail with `EACCES`; A's own session still works; login, logout and
`--version` probes still work.

## L3 Network

Runtime workers already run with `RestrictAddressFamilies=AF_INET AF_UNIX
AF_NETLINK` (`ipv4_only_v1`). Native login and logout start through
`authentication_process`, outside any unit. They get the same policy from a
seccomp filter installed in `pre_exec` that makes `socket(AF_INET6, …)` fail
with `EAFNOSUPPORT` (Go's resolver and dialer then use IPv4), together with
`PR_SET_NO_NEW_PRIVS`. The Machine refuses the login if the filter cannot be
installed. The Machine browser on OVH already resolves and connects through
the host's address families; its sign-in to Google must use IPv4 too:
acceptance checks the address Google reports in the browser
(`www.google.com` returns 200, not `/sorry`). If it does not, the browser unit
gains the same restriction. *(Open question 2.)*

## L4 Sign-in

- The account's Machine browser on OVH (`plugins/machine-browser`, already
  installed there) with `accounts.machine_browser` on and the account's
  `browser_enabled`. Each account card is its own browser identity; two
  Antigravity accounts can never share one (same family).
- The sign-in dialog of a `browser_code` login already offers only the
  account's own browser. Copy becomes Provider-neutral ("check that you are
  authorizing this account on the vendor's page" instead of Claude's), and the
  machine-browser policy's new-tab shortcuts add `antigravity.google`.
- The OAuth redirect lands on `https://antigravity.google/oauth-callback`,
  which shows the code; the owner copies it from that browser into the dialog
  (remote clipboard of the viewer). The Google session cookie remains only in
  that account's browser profile.
- Native login runs `agy-launch --cowboy-login` on OVH under L2 and L3.

## Phase 4: split execution on OVH

`agy` cannot delegate built-in tools to ACP `fs`/`terminal` (Phase 1), so:

**Launcher becomes an ACP proxy.** `components/provider-runtime/packages/
antigravity-acp/launch.mjs` (`node_npm` recipe, with `cowboy-launch.mjs`),
replacing the shell launcher's runtime path (login/logout keep the shell
modes). It spawns `agy acp --disable-telemetry`, relays ACP, and rewrites
`session/new`, `session/load` and `session/resume`:

- adds one stdio `mcpServers` entry `cowboy` (the facade, below);
- sets `_meta.antigravity.systemPrompt.append` to the projection;
- in split mode keeps `cwd` as the target path (the mirror directory on OVH).

Local (Hawk) sessions keep today's behavior: no facade, built-in tools, the
unattended/subagent hook.

**Facade MCP server.** `facade.mjs` over the shared
`components/provider-runtime/execution-connection.mjs` (`Connection`, executor
0.159.3), tools named and shaped like `agy`'s own:
`ReadFile`, `ListDirectory`, `SearchFiles` (target `rg`), `WriteFile`,
`EditFile` (exact replace, optimistic read-check-write), `RunCommand`
(`process/start`, bounded output, timeout, cancellation through
`process/terminate`). Paths are resolved against the binding's cwd; no path
escapes the workspace the target allows. A lost mutation reply is reported, not
retried (`execution-connection.mjs` rule).

**Hooks in split mode.** An allow-list: the `PreToolUse` hook denies every
tool except `mcp_cowboy_*`, `ask_question` and `manage_task` (and
`search_web`, which is network-only). The hook script is `hook.mjs` run by the
bundled Node, so it parses the tool name instead of matching a regex.

**Projection.** `systemPrompt.append` carries: the target cwd, OS, shell, Git
branch and `git status --short` (bounded), the facade instructions ("use only
the `cowboy` tools for files and commands"), and a bounded snapshot of the
target's `AGENTS.md`/`GEMINI.md` chain from the repository root to cwd
(Phase 1: `agy` reads these from its runtime cwd, which is empty on OVH).

**Readiness probe (zero inference).** Before relaying `initialize`'s reply the
proxy opens `Connection` (checks executor version, OS, cwd), lists its own
facade tools, verifies the hook file and Landlock domain are in place, and
writes the projection. Any failure refuses the session.

**Declaration.** `provider.execution-jsonrpc.v1` plus
`behavior.execution = {interface: "jsonrpc_v1", executor_digests:
["sha256:8bf204b3…"]}`, only after the native acceptance below passes on an
exact generation. Host: install on OVH, `--provider-runtime-machine
antigravity=ovh`.

**Native acceptance (test account).** Fresh, `session/load` and compacted
sessions each dispatch facade tools; a built-in tool attempt is denied;
implicit filesystem access by `agy` on OVH finds nothing; cancellation stops a
remote command; target disconnect fails the turn without local fallback; a lost
`WriteFile` reply is reported.

## L5 Account controls

**Per-account admission.** `accounts.antigravity_admission` (boolean) is
replaced by `accounts.antigravity_admitted` (list of account IDs, default
empty). Login start and session creation both require membership. The test
account is admitted first; each real account is added by the owner.

**One active turn per account.** The Controller refuses to dispatch a prompt
to an Antigravity session while another session of the same account has a
turn in flight; the prompt stays queued and the session shows "waiting for
<account>". Default and maximum 1 for Phase 4.

**Backend refusals.** Phase 1 showed refusals arrive as ordinary agent text
with `stopReason: end_turn`. The proxy instead watches `agy`'s own structured
log (`$HOME/.gemini/antigravity-cli/log/acp-server-*.log`) for the backend
status of the turn and, when it sees one, ends the turn with a fixed ACP error
whose text the signed `error_rules` classify:

| Backend signal | Classification | Account effect |
|---|---|---|
| `PERMISSION_DENIED … VALIDATION_REQUIRED` | `account_verification_required` | blocked until the owner completes verification in the account's browser and clears it |
| `PERMISSION_DENIED … UNSUPPORTED_LOCATION` | `account_location_unsupported` | blocked; points at L3 |
| ToS enforcement ("disabled in this account for violation of Terms of Service") | `account_suspended` | suspended: workers stopped, no retries, admission removed until the owner acts |
| `RESOURCE_EXHAUSTED` / 429 | `account_rate_limited` | backoff (no automatic retry; next owner send after the window) |

Core gains: classification → account state on the Controller, worker stop for
`account_suspended`, and the state on the card and picker. No fallback to
another account in any case.

## UX

**Account card** (Antigravity): name, `Antigravity · <name>`, Machine (OVH),
state (Not signed in · Signed in · Verification required · Location
unsupported · Rate limited until … · Suspended), "In use by 1 turn" when
busy, usage "Not available" (no documented surface), the notice "Use of a
personal Google account in Cowboy is not supported by Google", buttons Sign
in (opens the dialog → account browser), Open browser, Sign out, and
Admitted/Not admitted.

**Sign-in dialog**: state of the native login, the account browser entry, the
code paste field; no link outside the account browser.

**New session picker**: one entry per admitted, signed-in Antigravity account
with its state; never preselected; busy and blocked accounts disabled with the
reason. No default account entry.

**Session header / Settings**: the account name and state.

## Rollout

1. Implement L2, L3, L5, the proxy and facade; gates and Codex review.
2. Release; OVH maintenance; install antigravity on OVH; admit the test
   account; acceptance (L2 canaries, L3 address, sign-in, Phase 4 native
   acceptance, controls).
3. Admit real account 1; soak 1–2 days of ordinary use; then 2, 3, 4.

## Open questions

1. Extend `account_home_v1` to every Provider so no agent can read another
   Provider's or account's credentials (recommended next).
2. Whether the OVH Machine browser needs an IPv4-only restriction.
3. The Hawk local mode remains for the test account only, or is removed once
   OVH works.
