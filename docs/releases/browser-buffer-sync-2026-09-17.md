# Browser buffer synchronization delivery — 2026-09-17

**Published on main and activated as Web only.** The
[core browser synchronization owner](../plugin-browser-buffer-sync.md) adds
original-content preparation, one-use confirmation, explicit retirement and
bounded Settings presentation of uncertain operations. Buffer cleanup shares
its fence. This adds the client API and retained-operation surface, not an
ordinary Review refresh entrypoint, a Machine/Plugin upgrade or completion of
the refactor.

## Immutable inputs

Clean implementation source:
`f893f909d916cf2d1e898ac563b461cf0b27b972`, descended from remote/active Web
`57bb26c2491a185bc2e428f7bce00f60c9100e89`. The latest independent Composer,
screenshot and prompt-overlay fixes are preserved. Rust changes are one
test-only Service serialization fixture; no production Controller, Machine,
native, SDK, Plugin/component source, lock or durable format changed.

- Web release: `/nix/store/cnwkm9ndcfcnhpv1drpwy5yp9k4wvpx1-cowboy-web-release`.
- Web assets: `/nix/store/whldybxhxj8rwip3zlaxbc86apxggynp-cowboy-web-0.1.0`.
- SPA version: `1d89b04adab13a4b4df39e4b162c3fc1`.
- Service worker: `cowboy-v1705`.

The enclosing documentation follow-up only records verified evidence; it
does not change the deployed application's bytes.

## Gates

The complete pinned-shell
`nix develop -c env RUST_TEST_THREADS=1 just check-compact` passed on that clean
commit, including formatting, strict Clippy, Web types/lint, dependency audits,
feature/Plugin/component checks, differential composition conformance and
release builds:

| Suite | Passed |
| --- | ---: |
| Main Rust | 1,410 |
| Standalone Machine | 333 |
| Core Code adapter | 26 |
| Private Zed adapter | 75 |
| Web | 1,676 |
| Isolated PostgreSQL | 18 |

Main/Machine/private-Zed suites retain 32/two/two explicitly ignored tests.
The full gate is not acceptance of optional immutable/native flows or physical
devices. Existing dependency-policy, Web lint and large-chunk warnings remain
visible. The separate immutable Nix Web build also passed.

The 21 new unit cases bring the focused browser-buffer suite to 77. A shared
nonempty Rust/Web fixture proves closed Service serialization, real UTF-8
capture hashing and exact native-vector decoding. Compile-only cases reject
forged captures/confirmations, exchanged ID domains and arbitrary purposes.
Pre-auth local projection does not discover identity, open storage or send HTTP.

All 38 real Firefox `151.0.1` cases passed from the committed source, in fresh
profiles/private loopback namespaces with no production credentials:

| Browser suite | Cases | Fixture SHA-256 |
| --- | ---: | --- |
| Synchronization | 8 | `9b92537f08d4ec4ee5d708f788bd64714362b32d7176bc1eef2e91a0550e690d` |
| Buffer/content owners | 8 | `b78386233c49f102b09ceb9ce5f8d945edc15af71d7bfcaebba2182826b2edd7` |
| Product context | 6 | `671fc16594ce3c0f5483ad4f26081113a76bdfbd3dd33bdfa00288378bd106f3` |
| Buffer cleanup | 7 | `f5066b98c54928278091188e23cb730b532ed98a46691187643a92300199a1bc` |
| Settings recovery | 9 | `a1b1f6e79a93f987b545523117922137a95dfd2a60aabe05872418bf5659246f` |

The new suite exercises real React/MUI/StrictMode with deferred fixture HTTP:
passive mounting, cancellation/double confirmation, detached Apply/Query/Retire,
lost replies and 404, same-stack authority loss, stale previews, late preparation
after close, twelve long-path rows paginated at 360px, and same-buffer operation
replacement. These are not native-process or physical iOS/WebKit acceptance.

## Activation and continuity

Machine-owned transaction `1789614618519886638-f893f909d916` committed with
`outcome=succeeded`, `phase=committed`, `published=true`. No maintenance or
recovery override was supplied. The predecessor was
`/nix/store/iqlhyppqaxza4cg6pyd0mn9qg0n5k9yp-cowboy-web-release`.

The **11:09:21–11:10:33 +08:00** pre/post observation window retained all
**16 worker** and **four native Code** PID/start-time pairs. Controller,
resident Machine and Victoria processes, non-Web profiles, host/cold-recovery
files, workspaces, installed Plugin identities and failed-unit sets were
unchanged. Persistence remained healthy with zero dropped/failed batches.
This bounded continuity observation is not native generation-swap acceptance.

Machine remained online on `worker-4208d4d141de95cf9feb`; installed Zed remains
`1.8.0`. This delivery does not activate protocol 20 or publish/install `1.9.0`.
Ordinary Review still does not construct these synchronization owners, so
Settings has no new rows merely because this bundle was deployed.

Local/public `/healthz` and `/version` passed. Index, admin, service worker and
both entry assets match the immutable Web output byte-for-byte, with no-store
HTML/SW and immutable hashed-asset cache headers. An existing PWA must adopt
the new bundle; server activation alone is not evidence of its reload.

The initial dispatcher invocation supplied the internal-only `--machine` flag
and was refused before transaction dispatch; the unit was inactive and no
in-progress transaction existed. The corrected public invocation dispatched
the single committed transaction above. An initial evidence collector also
needed Deno's explicit `/proc` access; neither preliminary failure is counted
as successful activation or product-code acceptance.

Private scratch evidence: `/tmp/cowboy-browser-sync-VkaN75Qc` (not a permanent
public artifact). Selected SHA-256:

- Complete gate: `6b04e896acf917d878425ae229a5cec30944443c2b978ab2420eb155bdacf5f0`.
- Browser gates: `da506f259b372dc3e0bb4d9bea4a88ac2d86d90e16606734c0a79db4d014fef3`.
- Nix build: `fe8b585d19b78b6598cb40a777663b36bf9b1269e224b36a37e2aa9676c1ed08`.
- Rollout audit: `935edbf4141b69c78b8e9ff81726cc58f79d32ee7c80983f2e1f5b90084eada0`.
- HTTP receipt: `a389de0e58fe11e877ca45d8af676aec0889d324e3fccc034e01b5626c4b1de7`.

Remaining: ordinary Review content/position lifetimes and navigation owners,
actual Machine/Code rollout, independently authorized restoration and
abandoned-browser/restart recovery, plus supported-device acceptance. General
typed resolution/state compatibility exits remain separately tracked in the
[completion ledger](../plugin-refactor-completion.md).
