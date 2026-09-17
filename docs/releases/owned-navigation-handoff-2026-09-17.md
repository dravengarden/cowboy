# Owned navigation handoff candidate — 2026-09-17

This accepts the Zed `1.11.0` **private source/candidate slice**, not public
navigation, production installation or component activation. It extends the
[original navigation candidate](owned-navigation-candidate-2026-09-17.md) with
exact destination handoff and actual nonempty-LSP process acceptance. The
[finite contract](../plugin-owned-navigation.md) keeps ambiguous acquisition
fenced and does not treat a pathname or serialized resource ID as authority.

## Immutable inputs

Clean candidate source: `31be72f4781ff104790edf3143bcc0159e395bab`. It includes
the independent upstream keyboard-diagnostics change. The later merge
`acad9f657d8f307d34ca9f63ee0ef467fcece09a` adds only the independent Review
Markdown-table change and its documentation; the complete source gate also
passes on that clean merge. These later changes and this acceptance document do
not replace the native/Machine bytes tested below.

Only Linux x86_64 is declared. The adapter and consuming Plugin advance together
to `1.11.0`; private server `1.0.0`, upstream Zed revision, third-party
dependency pins, shared component versions and historical registry entries are
unchanged.

| Candidate                            | SHA-256                                                            |
| ------------------------------------ | ------------------------------------------------------------------ |
| Zed adapter `1.11.0` ELF             | `60285454305f5b6576813be727c0b38dd8ab4dcabed92b7f20b6e8920bca4ec0` |
| Unchanged private server `1.0.0` ELF | `da41ec6baee1cbf714b809fcd912dd200be150c634cbe4a6ec6939e323da8131` |
| Machine release entrypoint           | `2a080a3b4359a69db10fee868c3dc85dee2a2c92a0a4afba7f478946b4f6c3ac` |
| Machine wrapped ELF                  | `b7212de9175a5c9483e9ff86201d696bdd65d75d5d25f9288a99b4486fd52a53` |

Immutable outputs:

- Adapter:
  `/nix/store/24n188lmvx0dii0wcz6hs4gbv0jvhhad-cowboy-zed-adapter-x86_64-unknown-linux-musl-1.11.0/bin/cowboy-zed-adapter`.
- Server:
  `/nix/store/lywsfx0zmq03ml9rjlqhhqw9dmp0gdvr-cowboy-zed-server-x86_64-unknown-linux-musl-1.0.0/bin/cowboy-zed-server`.
- Machine: `/nix/store/8wp5rrx9a07ypagmp5zb6p2b5iwkvc5j-cowboy-machine-release`,
  candidate worker generation `worker-9cf7484e7b0b3cd35a4b`.
- Machine wrapped ELF:
  `/nix/store/ir72qsx6rfgdrcd6lq7pli7s4g1rs0pg-cowboy-machine-0.1.0/bin/.cowboy-machine-wrapped`.

The canonical runtime builder passes its static dependency audit and closed
credential-free probes. Generated artifact URLs are candidate metadata, not
evidence that the binaries have been published at those URLs.

## Implemented and verified

`prepareNavigationBuffer` reserves an ordinary owner from one exact retained
destination. Open revalidates the original group, native ID, owner, content and
epoch, then atomically adds the independent pin without native or filesystem
I/O. Cancellation before commit remains effect-free; a lost reply can query the
original lease. Parent release before Open refuses, while an already opened
handoff survives parent release and removal of source/target paths. Same-path or
equal-content replacements cannot substitute for the original target.

The real LSP fixture exposed two production-code gaps:

- Native navigation shares targets but does not register them with language
  servers. Newly retained native IDs now register once during Execute. Pins and
  evidence are saved first; failure, cancellation or a changed epoch retains
  Unknown without another registration or query.
- The actual static pair can return locations before asynchronous target State
  and last Chunk arrive. Capture now waits, with a bounded deadline and source
  epoch checks, for those exact native IDs. Incomplete or invalid shares cannot
  become success; no pathname open or query retry fills the gap.

Nine new private tests cover atomic handoff, independent owners, cancellation,
disconnected observers, exact target identity/content, edit/undo ABA, shared
capacity, registration failures and late initial shares. The Machine source
denies all four private navigation commands before generic runtime selection,
including requests with a forged worktree. No core navigation grant or public
forwarding route is added.

The pinned `CARGO_INCREMENTAL=0 just check-compact` passes on both the candidate
and clean integration merge, with `RUST_TEST_THREADS=4` and the inherited
Provider-package override unset. Final counts: **1,429 main Rust, 340 standalone
Machine, 26 core-adapter, 97 private-adapter, 1,736 Web and 18 isolated
PostgreSQL tests**, plus three Codex bridge tests, SDK/component/package
isolation, composition conformance, formatting, strict Clippy,
dependency/type/feature checks and release builds. The candidate before the
independent Web merge had 1,731 Web tests. Opt-in process tests remain ignored
in the ordinary suite and are exercised separately below. Existing Vite chunk
warnings are not suppressed.

Three process gates pass against the exact native pair:

1. `zed-native-navigation-conformance` passes **four times** with the final
   static adapter/server. A test-only stdio LSP, launched by explicit absolute
   executable path, supplies five nonempty navigation kinds, two cross-file
   targets, duplicate locations and non-BMP UTF-16 ranges. The real private Unix
   socket loses an actual handoff response, queries that original lease without
   resending Open, releases the parent after deleting paths, then reads/releases
   the independent target. The fixture observes one open/close for each of its
   three documents. Existing native synchronization and plaintext navigation
   regressions run in the same isolated gate.
2. `zed-plugin-conformance` passes temporary signed installation, mixed
   ownership, uninstall drain, retained resources after path removal and
   original-generation reactivation. It does not publish or install a production
   release.
3. `code-buffer-connected-conformance` passes all **11** authenticated
   HTTP/control/native checks with the Machine above and supplied Controller
   `/nix/store/4yj5b7fpnmjb65jnrsls116g6shimg1g-cowboy-controller-release`
   (source `e8f2c249328964c9a22bd536bedbb80339ce27b6`). Receipt v3 reports
   `accepted: true`, `cleanup: true`, no failure, three connections, six held
   replies, one discarded reply and one cut. The real lost-Apply case waits the
   unchanged 40-second timeout; only one Apply is dispatched. Independent
   receipt audit checks all 11 unique results and rehashes seven executable
   records. These are supplied artifacts, not acceptance of actual production
   host roles.

The new LSP executable is test-only, with synthetic language answers and
isolated homes/network/PIDs. It is not shipped as a runtime dependency and does
not use ambient language tools or downloads. Observed fixture didClose events
and forced fixture teardown are not universal native CloseBuffer
acknowledgements, verified production reclamation or independent recovery.

## Failed attempts and boundaries

An earlier static candidate failed because the original target share had not
arrived; that candidate is superseded, not accepted. A preceding invocation used
a mistyped immutable server path and failed before runtime startup. The initial
whole-suite run also timed out in the existing telemetry deadline test's
destination-start wait. That unchanged test passes alone and in both final
four-thread complete gates. No telemetry code, deadline or assertion was
changed, and this record makes no claim to have fixed an intermittent scheduling
issue.

Still required before public navigation: core-issued acquisition authority and
original-generation Machine routing, Service/principal/Session ownership,
destination text/view lifetimes and actual consumer/device acceptance. The
native query may allocate resources before adapter result limits reject its
answer; there is no global native allocation budget or OS filesystem-isolation
proof. Ambiguous acquisition retains the original evidence and a process-wide
admission fence; no independent saved-query recovery exists. General graph/state
leases and independently authorized post-effect restoration also remain open.

Following the canonical release skill, this task only verifies immutable
candidates and temporary signed fixtures. It performs **no production Catalog
write, Plugin installation or component activation**. It makes no claim about
concurrent host changes by other tasks.

Raw local evidence is under `/tmp/cowboy-navigation-handoff-aW0EBqvQ`:
`check-final.log`, `check-integrated.log`, `runtime-build-final.log`,
`machine-build-final.json`, `machine-build-final.log`, `native-pair-final.log`,
`native-pair-repeat.log`, `process-gates.log`, `connected-input.json`,
`connected-receipt.json` and `receipt-audit.log`. Earlier attempts remain in
`check.log`, `telemetry-rerun.log` and `native-pair*.log`. The accepted
connected receipt SHA-256 is
`74685a676c6d4ab94698c66ee122d973115b4ceeee96b9ebacb68fa4444f991b`.
