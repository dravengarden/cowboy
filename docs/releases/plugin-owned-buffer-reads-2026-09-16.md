# Original-owner buffer observations — 2026-09-16

The additive [diagnostic and symbol API](../plugin-owned-buffer-reads.md) is
published on main and active in the Controller. Machine support and private Zed
`1.4.0` are verified source/runtime candidates, **not a production installation**.
Review still uses its existing API. This is not completion of the Plugin
refactor, edited-buffer coordinate support or independent post-effect recovery.

## Scope and protocol repair

Reads borrow the original confirmed-open owner across Controller, Machine and
native lifetimes. The Controller rechecks the original credential, role, Session
incarnation and connection before both remote calls and at response delivery.
Core validates the closed operation, exact reference and bounded typed result.
No browser-supplied path or native reference selects a replacement resource;
read failure cannot release the owner or replay its effects.

Real pinned-Zed conformance exposed an existing protocol error: diagnostic
refresh receives an acknowledgement, while diagnostics arrive as buffer
operations, not `LspQueryResponse`. The adapter now observes those updates,
acknowledges their transport requests and distinguishes unobserved diagnostics
from an observed empty set. It bounds native base text and diagnostic storage,
rejects older per-server observations and invalidates coordinates on observed
edits, undo or reload. Native transport failures no longer become empty success.
The original open vector remains a lower bound, not a current-content certificate.

## Source and immutable artifacts

- Integrated source: `ba07f7f8a1eb52d71c8b91f7636227af39fa98b7`, clean and published.
  This includes remote main through `ac7e04b0`, preserving the separately released
  Claude plan-usage work and streamed-message correction.
- Controller release:
  `/nix/store/gxxwkjfbh1i1i70mna0p1mgsm1vzqrsr-cowboy-controller-release`.
- Controller executable:
  `/nix/store/5zrc30i45whjjw6azmq5f41x0r82fsqf-cowboy-0.1.0/bin/cowboy`.
  SHA-256: `b8dba18e0438d4a8f61ec199a2caf54e02da2c3746637f21f452429d769fa1ca`.
- Static Linux x86_64 adapter candidate:
  `/nix/store/05v3rxg2x01jkvamkfqmgjmmpdx7gi8d-cowboy-zed-adapter-x86_64-unknown-linux-musl-1.4.0/bin/cowboy-zed-adapter`.
  SHA-256: `852b0ba80d3af900842ea1a9ad1a441d6a11cde6728462c945c0aea373367606`.
- Unchanged pinned server:
  `/nix/store/xjhjhq461q2qfwir9vmwbnaq7qxp3v11-cowboy-zed-server-1.13.0/bin/cowboy-zed-server`.
  SHA-256: `5829fe9d9f0b7a5a27129dc217cc9954c3b4334da5da2426bffe55423e723ae5`.

Only the private adapter/Plugin version changes; no dependency pin, public SDK,
Machine protocol version, SQL migration or durable format changes. Building the
candidate is not a signed Catalog publication or installation. No registered
Machine, installed Plugin, production account or export policy was changed.

## Verification

The integrated pinned-shell `just check-compact` passed: **1,333 all-feature Rust,
305 standalone Machine, 26 core adapter, 35 private Zed, 1,510 Web and 17 isolated
PostgreSQL tests**. It also runs strict Clippy, formatting, feature/closure,
structural-link, dependency and shipped-build gates. All-feature and Machine
suites retain 29 and two explicit ignored tests respectively; the relevant
PostgreSQL and native conformance gates run separately. The immutable Controller
build passed its own 1,010 tests and three shim tests, with 17 explicit ignores.
Existing dependency-policy, Web lint/chunk and Nix cache/link warnings remain
visible; they were not hidden or repaired by mutating the host.

Thirteen core tests and ten private-adapter tests were added. They include real
temporary SQLite credential revocation/role changes at both remote boundaries,
cancellation, Session ABA, reconnect, busy release, retained runtime routing,
nonempty protocol vectors, diagnostic ordering/UTF-16 conversion, budgets,
reload invalidation and update acknowledgements.

`zed-plugin-conformance` passed with the exact static adapter and pinned server
from the integrated source: temporary signing and installation, legacy and owned
buffers, uninstall, deletion of the original file, worktree rename, both owned
read kinds, explicit release/drain and retained-generation reactivation. This
uses an isolated Machine store and plaintext file, not a registered Machine,
production login, nonempty real-LSP diagnostic result or distinct released
old/new Zed generation coexistence. Nonempty diagnostics use protocol fixtures.
The initial immutable conformance timeout is retained as failure evidence; it
led to the protocol correction above and is not counted as acceptance.

All four actual Controller roles (candidate, active/next recovery, retained
previous and cold) independently read the same **80 ready releases** and passed
byte-identical actual-Service configuration preflights. The six-Agent exact
publication-coverage gate passed, including Claude Code `3.1.24`. No Catalog
publication or authenticated refresh was performed by this task. The first
configuration preflight stopped at Deno's protected `/proc` permission check;
the audited local script passed with the required permission, without printing
the Service environment. Its failed log remains separate.

## Controller activation

Transaction `1789516913334745995-ba07f7f8a1eb` committed successfully with
`published=true`, using the installed machine-owned component activator.
The accepted predecessor/next recovery target was
`/nix/store/jps7bbmixfn7mj01v0mcg56x4i51cdfp-cowboy-controller-release`, source
`1e79fde41fe5a0d064e93f741142b26c1c530f59`. No recovery override was necessary.

In the bounded **08:01:30–08:02:23 +08:00** observation window (not an outage
measurement), only the Controller PID changed, `204353` to `267402`. All **13
worker** PID/start-time pairs, resident Machine and Victoria processes remained
unchanged. Machine reconnected online on `worker-3a889de3bf203a2378b8`, retaining
its workspace revision and identity hash. Web/Machine profiles and receipts,
host closure/unit hashes, cold roots and failed-unit sets were unchanged.
The pre-existing system failure `liveview-backup.service` remains recorded;
the user failed-unit set was empty. Neither was cleared by this task.

Local and public checks accepted `/healthz`, `/version`, exact index, admin,
service-worker and both entry-asset bytes and their cache headers. Web remains
source `902c401e`, SPA version `e967b101700b79a9882188f10f99a5d7`.
This Controller restart does not upgrade native code or switch Review.

Private evidence: `/tmp/cowboy-buffer-reads-LiEcgn5n`, including failed/accepted
native runs, complete gates, immutable builds, reader/configuration preflights,
before/pre-dispatch/after snapshots and activation/HTTP audit. SHA-256 values:

- Integrated complete gate: `a9855d76ec3f3d777dc99dd2c3f924c3859ecae6183586d553f63d70dbc1b8f3`.
- Immutable Controller build log: `b2ae2bc104717e56a378b4678b7dfb1207a6fdb5d83155da80e4d44212da98bc`.
- Integrated native conformance: `aef1ed690140abb0b1b45c397acc70b1b63d263fa212e38cb75b2fe8aadcb64e`.
- Four-role reader audit: `b55397c506a30a21f47fe29adb38b20eeb3844cd6bdbb840ef78b129fe93ad87`.
- Activation/HTTP audit: `f745c663a2033cca7c6eea83a3c0a23d207ee3ad504681222a150ee01e59cebc`.

The [completion ledger](../plugin-refactor-completion.md) still requires separate
Machine/Code rollout, client pending/unknown/release handling, genuine positional
content/anchor semantics, supported-device acceptance and independently
authorized recovery. An old Machine fails the new core support probe; this
Controller activation does not claim production end-to-end owned-buffer reads.
