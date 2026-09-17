# Content-owned working diff delivery — 2026-09-17

**Published on main and activated on Web only.** The
[working-diff consumer](../plugin-review-owned-diff.md) validates every new-side
line against bounded complete current-file content before mapping UTF-16
positions. Native reads compare that complete content on the original core
owner, never the patch alone. Deleted/header rows, incomplete/stale patches and
staged/history views cannot borrow current-file positions.

Changing the projection cancels old observers even when full source text is
equal. Explicit checks neither reopen nor reload native buffers; diff offers no
synchronization preparation or Apply. Owned failures never fall back to legacy
Code routes. This is a finite read consumer, not owned navigation,
native-generation acceptance, recovery or completion of the Plugin refactor.

## Immutable source and integration

Clean implementation: `e91f1072c53cc5692f9b45214dafc2b8ca0067ba`, rebased onto
remote `e8f2c249328964c9a22bd536bedbb80339ce27b6`. Integration preserves the
independent Review in-place refresh/scroll anchor and single attention row,
Sessions folders, Android changes and Machine broker fix. Routine diff states
take no status space. No SDK/Plugin version, dependency lock, native/core
protocol, durable format, migration or host policy was changed by this slice.

- Web release: `/nix/store/zdf0sb71h1l103idr4vpk2sjv09c7ri6-cowboy-web-release`.
- Assets: `/nix/store/5rgl4myxj6mqjcli83s6mnis42xzj63r-cowboy-web-0.1.0`.
- SPA version: `6f415d8b17c92b0d26331031af1d75e2`.
- Service worker: `cowboy-v1712`.

The later documentation commit records these bytes; no application redeploy is
needed for it.

## Acceptance

The clean source passed pinned-shell
`env RUST_TEST_THREADS=4 just check-compact`: formatting, Clippy, Web strict
type/lint checks, dependency and component/Plugin contracts, composition
conformance, feature slices, isolated PostgreSQL and release builds. The
immutable `.#cowboy-web-release` build also passed.

| Suite                   | Passed |
| ----------------------- | -----: |
| Main Rust library       |  1,423 |
| Codex app-server bridge |      3 |
| Standalone Machine      |    334 |
| Core Code adapter       |     26 |
| Private Zed adapter     |     75 |
| Web                     |  1,728 |
| Isolated PostgreSQL     |     18 |

Main/Machine/private-Zed retain 32/two/two explicitly ignored tests. Eighteen
new projection/paging cases cover every-line equality, EOF markers, Unicode,
bounded original cursors, cancellation and context loss. The projection-only
Deno typed test also accepts the non-constructible projection brand. A
supplemental Deno type-check of the loader's entire product import graph hit
existing state-sync/Deno-configuration errors; it is not reported as acceptance.
The project's strict TypeScript gate and normal Deno unit recipe both passed.
Existing dependency-policy, Web lint and large-chunk warnings remain visible.

All 55 actual Firefox `151.0.1` cases passed from the final source, in fresh
profiles and isolated loopback namespaces:

| Browser suite           | Cases | Fixture SHA-256                                                    |
| ----------------------- | ----: | ------------------------------------------------------------------ |
| Buffer/content owner    |     8 | `b78386233c49f102b09ceb9ce5f8d945edc15af71d7bfcaebba2182826b2edd7` |
| Product context         |     6 | `653d64f39d2ab7e58a84beec59c2b9ee3ee9fde0b885c2901e61fa058693b1f2` |
| Cleanup                 |     7 | `fa441c59c0fd0db8167e021eaf9a26263e802363822b1a53dced894d921d6536` |
| Synchronization         |     8 | `d091f646f8da46d65eb320e6197109afef42d3fa35d30a7ef45bb40a0cbe4c02` |
| Review source           |     6 | `62b74209ed3e7089dda188c9c44bf65ca54a2dc1fd205ee30f8e9fac05dda2d1` |
| Review working diff     |     6 | `80f6130de52f4b59c861d55320cae76b0c1505a9807988e19c248496b72854b2` |
| Review document refresh |     5 | `f200623f4e18ae372f1fd770e46a3130cbfc47f607d74094bbb4ba330a9afb2b` |
| Settings recovery       |     9 | `3ee7e654341760540751dc7f62efcbe9a7fc8bad44c0c15e0ebe2801b2015c35` |

The new suite mounts actual hooks, status controls and CodeMirror with
development StrictMode and WebCrypto. It clicks real deleted/new rows, assembles
split-CRLF pages, verifies full-file hashing, fences old hover on equal-source
projection replacement and refuses late replies after authority ends. HTTP is
synthetic. Its same-hook scope-change test does not claim that the product
bypasses its stricter Session/path/kind/scope document-remount key. The existing
actual DocumentView suite independently checks in-place refresh and
reading-position preservation. Neither is authenticated production or
physical-device acceptance.

Two fresh [connected native runs](../plugin-code-connected-conformance.md)
passed all eleven groups, schema v3, `accepted=true`, `cleanup=true`, bound to
the final harness source above. Supplied immutable artifacts:

- Controller:
  `/nix/store/4yj5b7fpnmjb65jnrsls116g6shimg1g-cowboy-controller-release`,
  source `e8f2c249328964c9a22bd536bedbb80339ce27b6`.
- Machine: `/nix/store/m68hbg8n34c7amz1aksqwqv14lhjz91b-cowboy-machine-release`,
  source `15a4c2496c83fc52064f92b2d6d8167aaf0cd7f3`.
- Native adapter:
  `/nix/store/k67bc55f5yrd4k8nlzri62m0z6kjg5rs-cowboy-zed-adapter-x86_64-unknown-linux-musl-1.9.0/bin/cowboy-zed-adapter`,
  SHA-256 `b9b5c5da9bf47e54a31bf807f6ac21387a3e9ed6a895da89b551e80016bdc5f3`.
- Native server:
  `/nix/store/lywsfx0zmq03ml9rjlqhhqw9dmp0gdvr-cowboy-zed-server-x86_64-unknown-linux-musl-1.0.0/bin/cowboy-zed-server`,
  SHA-256 `da41ec6baee1cbf714b809fcd912dd200be150c634cbe4a6ec6939e323da8131`.

The core pair matches the already-active production artifacts but runs only in
fresh private fixtures: real disposable login/enrollment, temporary signed
installation, independent owners, uninstall, one lost actual Apply reply with
the unchanged 40-second timeout, original-ID Query and separately settled
retirement. No production credential, installation or generation participates.
Forced fixture teardown is not product cleanup or recovery; plaintext queries do
not prove nonempty LSP or atomic diagnostic freshness.

## Activation and bounded continuity

Implementation was pushed before activation. Transaction
`1789627123649434874-e91f1072c53c` committed with `outcome=succeeded`,
`phase=committed`, `published=true`, replacing only Web predecessor
`/nix/store/28qzm4qyhjf2g403cfcixwpx3zggdx34-cowboy-web-release`. No maintenance
or recovery override was used.

The **14:38:31–14:39:09 +08:00** observations retain all **16 original worker**
and **two native Code** PID/start-time identities. Controller, Machine and
Victoria processes/profiles, host/cold-recovery files, workspace identity,
installed Plugin identities and failed-unit sets are unchanged. Persistence is
healthy, with zero dropped/failed batches in both observations.

Unlike the earlier source release's protocol-19 snapshot, Hawk was already on
the supplied protocol-20 Machine when this slice started. It remains online on
`worker-c7f0635a1884fdd1ac6c`. Installed Zed remains **1.8.0**, generation
`sha256:56474a7197fb8ba30d401236e780a35f107a9e9e7a5ab9869445c0d53a425d20`. This
task issued no Controller, Machine or native Plugin activation. The manifest
selection still does not grant support to an older native generation; each
native operation checks its own capabilities and fails without fallback.

Local/public `/healthz` and `/version` pass. Index, admin, service worker and
both entry assets match the immutable Web bytes; HTML/SW use no-store and hashed
assets immutable. Existing PWAs must adopt the new bundle. Server activation and
bounded process continuity do not establish that any actual authenticated client
reloaded or completed owned diff reads.

## Evidence and remaining exits

Private scratch evidence: `/tmp/cowboy-review-diff-fRjcn2XK`, not a permanent
public artifact. Selected SHA-256:

- Complete gate:
  `030aca5dfbe82c629f945d91b7d705cf6406018dfaebeb6afb32efdc08a4177e`.
- Browser gates:
  `0fe8f01d42a8df2000bd6f6e0db89d4f5e5a18ab569668b6e384574890fc2ae5`.
- Immutable Web build:
  `ba22699cc2c29ccd1759c6be970d098e46239da576c0bff1f0f55409cd8e7758`.
- Connected run 1:
  `21b67bfd06b13eb3f206931b464d18906d9714575ead438828f10601877b67ac`.
- Connected run 2:
  `8a220d08b87f1eaa567c55983e8097590bf3b8f7a1baefcba62b396eedd0f835`.
- Rollout audit:
  `397a9a8e0250e5cfd4c967518076e0252a24b4814cdfee180d1d4dbffe4fe6c4`.
- HTTP receipt:
  `d7d899be676c3f9e13c615a640ce6f2335ac96df84db32abf1f6184d928e0e28`.

Owned navigation destinations, separately authorized exact native installation
and actual consumer/device acceptance, abandoned-browser/restart recovery,
independent post-effect restoration and general graph/site/state leases remain
in the [completion ledger](../plugin-refactor-completion.md). Staged/history
coordinates are intentionally excluded, not made safe by a current-file hash.
