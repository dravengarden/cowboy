# Scoped file continuations — Controller release, 2026-09-15

The [code-read boundary](../plugin-code-read-scopes.md) now binds file-page
continuations to the original typed context and exact requested path before
local or remote I/O. Page ETags include the actual representation and cursor
lifetime. Shared local/cached readers repair UTF-8 page boundaries, incomplete
EOF and the 32 MiB view limit. This finite slice is published and active on Hawk;
it is not completion of the Plugin refactor or reversible execution.

## Source and activation

- Implementation: `327d10d4`; clean release source:
  `4587940328cc04f361ec65834dc3d3ae30767ac2`, published on main. The merge retains
  the independently published Claude Plugin preset commit `cb58ef71`.
- Controller release:
  `/nix/store/5yj4jq9p9ik3dy608xkbfryk08l3khjm-cowboy-controller-release`.
- Executable:
  `/nix/store/c3w90hp7ry869rjb0hnkrmd40bqg973d-cowboy-0.1.0/bin/cowboy`.
- Executable SHA-256:
  `36b1994bdc7bb3d1541d2dd4bc40ea8a393e73be9da3b16936334a07227edee4`.
- Transaction: `1789460762946766872-4587940328cc`, `outcome=succeeded`,
  `phase=committed`, `published=true`.
- Accepted predecessor:
  `/nix/store/dhzf0v3d370d6mqgpr4mza0hlvdgh3zs-cowboy-controller-release`,
  source `43f5db8515ec8e3e7e8377e994c5e40ea7b890d5`.

The installed machine-owned component transaction restarted only
`cowboy.service`. This task did not publish/install a signed Plugin, change the
Web release, Machine protocol, SQL migration, durable format, authentication or
telemetry policy, or activate a host/Machine generation. The browser cursor keeps
the opaque `64-hex:offset` shape; old unbound or expired tokens return `410` and
the existing Web reader reloads. Native adapter wire requests remain unchanged.
No extra data-recovery policy or migration is needed for this in-memory cutover.

## Verification

Both the initial and merged-source pinned `just check-compact` passed: **1,240
Rust library tests, 285 standalone Machine tests, 26 standalone Code adapter
tests, 1,481 Web tests and 17 isolated PostgreSQL tests**, plus 86 real-CLI
Rust/TypeScript structural-link vectors, formatting, strict Clippy, dependency
audits, independent feature/native-source/Plugin closure checks and shipped
builds. The standalone Code adapter library test graph is now part of the
standard gate. Existing ignored tests, Web lint/chunk warnings and the transitive
dependency-policy warning remain visible. The clean immutable Controller build
passed its own 935 library tests and three shim tests.

Twenty-one source tests were added. Coverage includes independent Hubs/Sessions,
cwd ABA and recreation, every advertised Workspace identity axis, exact paths,
changed offsets, native-cursor rejection, bounded identity bytes/count/idle time,
concurrent issue, invalid/nonprogressing pages and empty/limited terminal pages.
ETag tests cover exact serialized JSON, conditional matching, page distinctions,
expired/regenerated tokens and discarding the complete reply after scope drift.
A real Unix adapter fixture reconstructs a long Unicode file through the
opaque-to-native translation. Three regression tests failed before the repair:
cached and uncached long multibyte lines failed decoding, and incomplete EOF
produced a nonprogressing continuation. An initial test-harness compile error was
corrected separately; it is not counted as a product regression.

The independently built native artifact also passed **seven real process / Unix
socket cases**, using a closed environment and temporary workspace: long two-,
three- and four-byte-character lines, incomplete EOF, an empty file, binary
rejection and the view-limit partial tail. The exact artifact is
`/nix/store/sbrwqnc221ish3jfc8adiamvd7mdk1d9-cowboy-code-adapter-0.1.0/bin/cowboy-code-adapter`,
SHA-256 `3fa4d3117be359e3d6e95eb288ade1d7890e70716f0e5d9daf536dcfe79563cc`.
The disposable process was stopped. **This does not upgrade a resident remote
adapter**: that needs the independently authorized Machine release and native
acceptance. Controller activation updates colocated reads and Controller cursor
bindings for both local and remote paths; the signed Zed Plugin is unchanged.

No production private Code endpoint, account login, Plugin installation or
managed Victoria cutover was used as a test. Installation/telemetry journals and
policies are unchanged; their historical 807-role matrix was not rerun or
recounted here. The socket fixture is not native-session/generation acceptance.

## Production observations

The bounded pre-dispatch/after window was **16:25:41–16:26:48 +08:00** (67
seconds, not an outage measurement). Controller PID changed from `2188371` to
`2388350`; its running executable matched the accepted artifact. All **16
worker** PID/start-time pairs, resident Machine PID `1232222` and three Victoria
processes were retained. Machine stayed online on
`worker-48ad34f5c4615668b75f`, with workspace revision `5f38ce9d` and its advertised
workspace hash unchanged within this window.

Web/Machine profiles and receipts, host closure/unit hashes and cold roots were
unchanged within that same window. Both system and user failed-unit sets were
empty before dispatch and afterward. The earlier development baseline did
observe a different host/workspace revision and the failed user portal unit;
those changed **before** this transaction. This task did not activate that host
update or clear that failure, and does not attribute their change to this fix.
The dispatch used the installed component activator; the unrelated fetched
Columbus workspace change did not change its recipe.

Local and public HTTPS checks accepted `/healthz`, `/version`, exact index,
admin, service-worker and both entry-asset bytes, including cache headers. Web
remained on SPA version `9dcf7c01602e4bf519e697e619761b03`. This bounded process
evidence is not a physical-device test, atomic HTTP-delivery fence or native
generation swap.

Private evidence is retained at `/tmp/cowboy-file-page-scopes-RcCqdFIN`, including
the failed regressions, targeted/initial/integrated gates, immutable builds,
standalone adapter probe, before/pre-dispatch/after snapshots and activation/HTTP
audit. SHA-256 identities:

- Integrated complete gate:
  `94e7f818702af0b4b7a971eb5edc4a3d7de775ff96ed25f7c7868a1d33eaee0c`.
- Immutable builds:
  `1f0bfcc607b173396a27feaab0f23e5faa36f8733759ca0130ca50212a65ee6e`.
- Standalone adapter acceptance:
  `547b4bdffd7f7c591fd3a515297d562376bb192fd1e3a814589de3338c11e5b0`.
- Activation/HTTP audit:
  `d450be83242f5fec95a933b277c9f951a31c2365a694c38e584b4aefcecfccd3`.

Continuous Machine-owned Workspace identity, general state leases, independent
post-effect/native recovery and supported-device/account acceptance remain in
the [completion ledger](../plugin-refactor-completion.md). A read observation or
discarded stale response does not undo an already-dispatched effect.
