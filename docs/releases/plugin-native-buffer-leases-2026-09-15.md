# Prepared native buffer lease candidate — 2026-09-15

Status: implemented and verified, **not a Plugin publication or production
activation receipt**. Controller/Web continue to use the legacy buffer API.

## Source and immutable candidates

Implementation/build source: `c9771122a603e2709001bdac57c8bf8493ff875d`, clean,
rebased onto then-current `main` at `05fa1646`. Later integration of the upstream
Web-only usage fix `b12e3e60` preserves that source commit and its artifacts.

The Zed source Plugin/private adapter advances **1.2.4 → 1.3.0**, independently
of its component-release pin `2.9.0`. The exact server and all external dependency
versions remain unchanged. The adapter directly declares the already-locked
`getrandom` **0.4.3** for its process incarnation. A lockfile regeneration briefly
selected a newer cached `syn`; it was restored to the original **3.0.3** before
the accepted gate and immutable builds. No incidental dependency upgrade ships.

| Candidate | Immutable identity |
| --- | --- |
| Portable adapter | `/nix/store/bc9hrxw8vhkd9wwwq4ab9p024qa8niki-cowboy-zed-adapter-x86_64-unknown-linux-musl-1.3.0` |
| Adapter SHA-256 | `26d1c75c553b806a3115cb0f109ceca679ea31d5998dadace44595f52fe7410d` |
| Server 1.13.0 | `/nix/store/xjhjhq461q2qfwir9vmwbnaq7qxp3v11-cowboy-zed-server-1.13.0` |
| Server SHA-256 | `5829fe9d9f0b7a5a27129dc217cc9954c3b4334da5da2426bffe55423e723ae5` |
| Machine release | `/nix/store/dlnfvfg0y2949s2ry5d7hbh5rld9ngp4-cowboy-machine-release` |
| Candidate worker generation | `worker-3a889de3bf203a2378b8` |

The source Zed package covers Linux x86_64 only. Its portable adapter and server
have neither an ELF interpreter nor `NEEDED` shared libraries; the package-owned
builder verifies this and runs isolated, credential-free probes. The Machine
release is a separate Nix artifact, not a portable Plugin payload or an activated
worker generation.

The bound **unsigned** candidate remains under `dist/plugins/zed`:

- package digest: `sha256:43de903fb1ea0e22c0d826b8b708829857ea8f1f589eaece614d830237f326cc`;
- composite digest: `sha256:fb6bd36be515417ee2bcd73ba4ceeb5591c5730df05d9caceec212afeb9ff09a`;
- contract fingerprint: `sha256:76ba0050dd62452d50629c5f9041a669241d1fa85793a9e536d7402d0f59eac2`.

Its publisher field names `cowboy-first-party`, but its signature is empty.
Generated content-addressed URLs are intended destinations, **not** published
availability. The temporary conformance signature does not sign this candidate
for production. Before publication, independently sign/verify it and accept the
actual live and recovery Catalog readers; then verify exact Catalog availability.

## Accepted evidence

Proof directory: `/tmp/cowboy-owned-buffers-q76PD51W`.

- `just plugin-check` and `just check-compact` passed at the integrated build
  source. The complete gate includes strict Rust lint, dependency policy,
  independent feature builds/tests, Web checks and release builds: **1,297**
  all-feature Rust tests, **300** standalone Machine tests, **26** core Code
  adapter tests, **25** Zed adapter tests and **1,483** Web tests passed.
  **17** isolated PostgreSQL cases also passed. The ordinary gate explicitly
  ignores 29 all-feature / 2 standalone process-specific cases; the selected
  real Zed case was run separately, not counted as silently covered.
- After merging the upstream Web-only fix, `just plugin-check` and the complete
  `just check-compact` passed again at `d15ddcef`: the Rust/native counts above
  remain unchanged and **1,495** Web tests pass. `check-merged.log` records this
  final integrated gate; the earlier `check-integrated.log` belongs to the
  immutable candidate's build source. Later edits only clarify documentation
  and command comments, not the tested native/Machine implementation.
- **29 new tests** cover native identity/non-reuse, path-free release, worktree
  incarnation and symlink refusal, native/legacy owner separation, closed wire
  shapes, missing active owners, socket-observer loss, unknown/cancelled effects,
  capacity and effect-free expiry. Machine process fixtures additionally cover
  original-generation retention, dropped open/release replies, wrong references,
  cancellation-safe cleanup, dead-process refusal/descendant reap, independent
  generations and the core-only support probe.
- `zed-plugin-runtime-build` produced the exact static adapter/server pair from
  clean committed source. `plugin-build`, artifact-URL binding and runtime-matrix
  binding succeeded through the shared SDK.
- `zed-plugin-conformance` passed with those exact real binaries in a private
  user/network namespace containing only loopback. It temporarily signs and
  installs a package, opens legacy and owned buffers, checks duplicate opens and
  read-only state, uninstalls, rejects a new worktree, drains legacy leases,
  deletes the owned file, renames the worktree, releases both original handles,
  verifies retained-generation drain and re-verifies/reactivates retained bytes.
  It sends no inference prompt, uses no Service credentials and installs on no
  registered Machine. This is production Machine code exercised by its test
  harness, not a connected immutable Controller/Machine rollout matrix.
- The independent Machine Nix release built successfully. No component activator
  was invoked, no production package was installed and no service was restarted
  by this task.

The first extra invocation of `plugin-runtime-probe` rejected the Code payload
before any runtime started: that Python harness is Agent-only. It produced no
acceptance receipt. The canonical release skill and command documentation now
say this explicitly; Code acceptance above uses its own required real-binary
gate, not a bypass or an Agent fixture. Earlier local lint found one test-helper
ownership warning, fixed before the accepted strict gate. Failure logs remain
separate from accepted evidence.

## Actual installation and remaining boundary

Read-only Hawk inspection found its Zed active installation link at
`sha256:b5695b999b6be2c396bb22c74079a8c217b064874dd04f53f1eeedb838b2e699`,
whose installation inventory records **1.2.2**, not the source baseline 1.2.4.
This observes installed metadata, not the identity or behavior of a running
legacy native process. No live slot, legacy route, Catalog, authentication,
worker, host policy or installation journal was changed by this task.

Before ordinary client use, implement the core Controller owner/continuation,
bind it to original Service/principal/Session/connection identity, wire the Web
and language-query lifetimes, and accept the core Machine support probe plus
exact native generation. Then perform the separately authorized Machine
maintenance and exact Zed upgrade, including legacy drain and supported-client
acceptance. A Controller release alone does not activate this candidate.

`released` means local native ownership removal and close enqueue when needed;
Zed's close protocol has no ACK. It is not compensation, file-edit undo, full
worktree-cache reclamation, independent progress through arbitrary native stalls,
durable cross-process recovery, or restored Agent-session evidence. General DAG,
state-lease and post-effect recovery exits remain open in the
[completion ledger](../plugin-refactor-completion.md).
