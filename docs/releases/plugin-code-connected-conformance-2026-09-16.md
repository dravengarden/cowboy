# Connected core Code acceptance — 2026-09-16

**Source-only acceptance tooling; no production activation.** The new
[connected gate](../plugin-code-connected-conformance.md) exercises real
immutable Controller/Machine/Zed processes across authenticated HTTP and the
enrolled control connection. It complements the separate browser-owner and
native-runtime gates, not ordinary Review integration or recovery.

## Source and supplied artifacts

Final implementation source:
`8aefb03240353cf1b2836d7c60ab8ab8eb32c9d4`, integrating remote main
`435adb3c98dc097526e1edda3eb082eca0b2c0a0`. The first passing run used
`4f8a57e0`; the second used the integrated clean source. The enclosing
documentation commit only records evidence. Other than formatting an incoming
Provider test assertion, Rust changes are confined to test modules. No public
Plugin contract, dependency pin, migration, runtime implementation or Web
bundle was changed by this slice. The canonical release skill now requires
the connected gate before an owned-buffer/Review cutover.

The supplied component releases both identify source
`5ea1b6eea44656eed3bd9003723c5213c2cd6009`:

- Controller: `/nix/store/i4jccmj6dn3n0llzwgay4y66dbl05j03-cowboy-controller-release`.
- Machine: `/nix/store/cjca68ivnxrrpgfyb5rbxxszvj0f861q-cowboy-machine-release`,
  worker generation `worker-42bf5a39006ffed81a1d`.
- Private Zed adapter: `/nix/store/c9rmd5rx6bfw131l3zcm7cri62k477f4-cowboy-zed-adapter-x86_64-unknown-linux-musl-1.6.0/bin/cowboy-zed-adapter`.
- Zed server: `/nix/store/xjhjhq461q2qfwir9vmwbnaq7qxp3v11-cowboy-zed-server-1.13.0/bin/cowboy-zed-server`.
- Core file adapter, derived from that Machine release:
  `/nix/store/ys493kl6sl5dpaz63x1ma7rak6nrhksp-cowboy-code-adapter-0.1.0/bin/cowboy-code-adapter`.

These are supplied immutable artifacts, **not** claims about active/recovery/cold
host roles. Receipts bind provenance, wrapper/executable hashes and pinned Git
and SSH signing helpers. The source's Zed package hash is
`ae4779cae46bfc53afc1c34338fdd763c09399fc46a5e16bc4373549b6721df5`.
Each run signs its own disposable envelope; none is a Catalog publication.

## Accepted evidence

Both actual-process runs passed all seven check groups with `accepted: true`
and `cleanup: true`. The integrated run observed three authenticated
connections, three matching runtime configurations, 57 correlated replies,
three held replies and one intentional connection cut. Exactly three native
opens, two releases and one uninstall step were dispatched. Dropped HTTP
observers and duplicate requests did not replay those effects.

Three independent owners share the opened text without sharing release
authority. Language/symbol/hover reads check exact UTF-8 content, including a
non-BMP character and its UTF-16 hover position. Changed disk bytes mismatch;
the original owner remains readable after genuine HTTP uninstall, file deletion
and worktree rename. A borrowed read blocks release until it drains, followed
by an explicit new release request. A replacement connection cannot adopt the
remaining owner. Controller restart returns `404` for that process-local ID,
not a restored resource or successful release.

Native teardown deliberately kills only fixture-owned executables and reaps
their descendants. This last unresolved owner makes **forced fixture cleanup
distinct from product release, drain, rollback and post-effect recovery**.
The private PID/proc namespace and empty read-only cgroup mount protect the
resident Machine; temporary password, signing key and source text never enter
receipts. No production account, host policy or installation was used.

The complete `nix develop -c env RUST_TEST_THREADS=1 just check-compact` passed
on the same clean integrated source: 1,351 main Rust tests (31 ignored),
308 Machine tests, 26 core adapter tests, 56 Zed adapter tests,
1,599 Web tests and 17 isolated PostgreSQL tests. It also passed the 86-case
composition differential gate, package/Provider checks, native-shell contracts,
Clippy, dependency audit, feature checks, type checks and production builds.
The ignored connected gate was executed separately, not counted as a unit pass.

Earlier failing runs exposed missing fixture Git/core-adapter setup, an
incorrect expectation that an admitted mutation stayed `prepared`, a missing
uninstall-preflight relay case and incomplete orphan reaping. They are retained
as failures, not acceptance or evidence of a production regression.

Private scratch evidence: `/tmp/cowboy-code-connected-yEs78P3v`, not a permanent
public artifact. Selected SHA-256 values:

- First passing receipt (`run5.json`):
  `69688074d0b39b8b85fb22f28016d5ccbc17b4053b10514f2c4da3a1e8766562`.
- Integrated-source receipt (`integrated.json`):
  `01732708348c3c186a6f83bdcbb0ff3dd05e9860e90a64585360f683ad810157`.
- Integrated-source process log:
  `ef920a309d669446695d8aca3882b8059de7cc0bbf1215088a5708fdc1598369`.
- Complete source gate:
  `fd288ac69d56cc484e4a11783d88c3b7cfcb42b1e0b7e73bb9a6b1e1e0c61666`.

## Remaining boundary

Still required: connected Code installation admission, ordinary Review, explicit
disk/native synchronization, owned navigation destinations, nonempty LSP/fresh
diagnostics acceptance, separately authorized and accepted Machine/Zed rollout,
abandoned-browser/restart recovery, post-effect restoration and supported
devices. No production process was restarted, and no production Plugin
installation was changed by this task.
See the [completion ledger](../plugin-refactor-completion.md); this is not whole
Plugin-refactor completion.
