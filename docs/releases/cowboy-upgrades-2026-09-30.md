# Cowboy dependency upgrade — 2026-09-30

This batch updates the supported core dependency graph, build toolchains and
independently versioned Agent Plugins. Publication, component activation and
Machine installation require their own receipts; source versions alone are not
evidence of a production upgrade.

## Released source

| Area                 | Previous | Released    |
| -------------------- | -------- | ----------- |
| Rust build toolchain | 1.97.1   | 1.98.1      |
| Core build Node      | 24.15.0  | 24.21.0 LTS |
| Private Agent Node   | 24.19.0  | 24.21.0 LTS |
| React / React DOM    | 19.2.6   | 19.3.0      |
| TypeScript           | 5.9.3    | 7.0.2       |
| Vite                 | 8.2.1    | 8.3.1       |
| Vite React plugin    | 5.2.0    | 6.1.1       |
| Oxlint               | 1.67.0   | 1.86.0      |
| Mermaid              | 11.16.0  | 11.17.2     |
| Prettier             | 3.9.5    | 3.9.9       |
| SQL formatter        | 15.8.2   | 15.9.0      |

Cargo resolves 159 compatible package updates while preserving the SDKs' 1.97.1
minimum compiler. Rust 1.98 detects redundant trait imports and fixed-size chunk
iterators; those are corrected without changing protocol or storage behavior.
Six HTTP helpers deliberately retain their owned Axum rejection response and
document that local lint expectation. The dependency policy permits exactly
WebAuthn's base64 0.21.7 alongside the modern HTTP stack's 0.23; advisory,
license and source checks still cover both.

TypeScript 7 removes `baseUrl`; all path mappings are now explicitly relative.
The application already uses Vite 8 without Babel transforms, so the React
plugin's version-6 migration requires no compiler feature opt-in. References:
[TypeScript 7 migration](https://devblogs.microsoft.com/typescript/announcing-typescript-7-0/)
and
[Vite React changelog](https://github.com/vitejs/vite-plugin-react/blob/main/packages/plugin-react/CHANGELOG.md).

## Independent Plugins

| Plugin          | Previous         | Released | Private dependency changes                            |
| --------------- | ---------------- | -------- | ----------------------------------------------------- |
| Claude Code     | 3.1.34           | 3.1.35   | CLI 2.1.285, ACP 0.84.0                               |
| Claude DeepSeek | 3.1.24           | 3.1.25   | Same Claude pins; gateway unchanged                   |
| Codex           | 3.1.28           | 3.1.29   | CLI 0.159.2, ACP 2.0.1, rebased owned transport patch |
| Codex DeepSeek  | 3.1.24           | 3.1.25   | Same Codex pins; gateway unchanged                    |
| Gemini          | 3.1.24           | 3.1.25   | CLI 0.62.0                                            |
| Grok            | 3.1.25           | 3.1.26   | CLI 1.0.44                                            |
| Zed             | 1.20.0 installed | 1.20.2   | SDK closure from 1.20.1 plus compatible iterator fix  |

Shared runtime component 1.1.8 has append-only registry release 3.31.0.
Authentication contracts and model presets remain unchanged. Exact registry
archives and integrity values, including Node's published checksums, were
verified before mutation. The two private gateways remain pinned to Columbus
`27352e3400e5b55c6c88d6e7ac80f788e12dfb53`; their source directories have no
newer changes in the freshly fetched default branch.

Codex ACP 2.0.1 changes its input reader. The owned patch preserves bounded
strict UTF-8 framing, redacted failures, explicit pending-request cleanup and
configured native executable arguments. It also retains upstream fragmented
Unicode, valid final-frame and linear large-message tests. Typecheck, build and
all 1,027 enabled upstream tests pass; 33 upstream tests remain skipped. The
packaged worker, generation coexistence and cold/warm resume checks are separate
release gates.

GitHub 0.1.0's previously blocked publication is complete; see the
[accepted host-floor and publication follow-up](workspace-extensions-2026-09-29.md#publication-follow-up--2026-09-30).

## Retained versions and scope

- Zed upstream 1.21.0 rejects all seven current private server patches with zero
  fuzz. Retain the exact private upstream revision and server 1.6.0; accepting a
  newer upstream requires porting and revalidating the native acquisition, edit,
  ownership and resource bounds.
- MUI remains on 7.3.11. Its
  [v9 migration](https://mui.com/material-ui/migration/upgrade-to-v9/) removes
  component APIs and changes supported browsers, focus and DOM behavior. That UI
  migration is not accepted by this compatible dependency batch.
- CodeMirror, Lezer, Vim and the editor wrapper keep their locked versions.
  Their coupled iOS caret/IME/paste/render/expand matrix remains required by
  `web/AGENTS.md`; Firefox evidence does not establish that acceptance.
- KaTeX and Mermaid keep their current major lines and renderer integration.
  Node stays on the supported 24 LTS line, rather than the current 26 line.
- The iOS device linker blocker remains: latest swift-rs is still 1.0.8. No
  device release or physical-iPhone caret fix is claimed.

## Evidence

Task evidence is retained in `/home/draven/tmp/cowboy-upgrades-20260930`. The
initial production snapshot records 18 ACP workers and the exact component
profiles. Runtime probes use temporary homes and fake authentication, never the
Service credential. Production upgrades use the existing delegated Operator and
preserve its saved operation identities and active-session fences.

The Firefox regression run passes 16 suites / 131 checks across workbench,
Plugin management, Review and IndexedDB. Full quality, signed runtime and
production rollout receipts are recorded after their completion below.

`just check-compact` passes on source `3b73dddb`: formatting, lint, source and
dependency policy, feature slices, native-shell contracts, Provider isolation,
composition and site gates, 1,654 core Rust tests, 392 standalone Machine tests,
27 core adapter tests, 126 private Zed adapter tests, 1,948 frontend tests, 17
disposable PostgreSQL checks and production builds. Existing explicitly ignored
integration cases remain separate conformance gates.

Rechecking the browser suites with Node 24.21 exposed a timing-dependent fixture
click after returning from resource detail. It now waits for the next-page
button to become enabled and asserts that every clicked control is enabled; all
original resource and stale-response assertions remain. The new fixture passes
the actual Firefox gate. This is a test synchronization correction, not a change
to the product's loading or request behavior. The pinned shell also explicitly
includes Nix, required by the private gateway builder.

## Final integration and acceptance

Production source is `ad3c076ce669ff6609847828d740c2957aa8aa83`, published to
remote `main`. It includes the concurrent main-branch installation recovery and
verified runtime-cache change `8d8dd3ce`. All three final component artifacts
were built from this clean merged commit. Plugin packages reproduced by the
merged source are byte-identical to the independently signed packages; their
original runtime build receipts remain authoritative.

The complete `just check-compact` gate passed again on the merged source: 1,659
core Rust tests, 396 Machine tests, 27 core adapter tests, 126 private adapter
tests, 1,948 frontend tests and 17 disposable PostgreSQL checks, plus
formatting, lint, dependency policy, composition, feature slices and release
builds. The final Node-24.21 Firefox run passed 16 suites / 131 checks.

All six Agent releases passed actual macOS arm64 execution and Linux worker
initialization, session creation, old/new generation coexistence and descendant
cleanup. Linux was checked against the original production worker and both
candidate workers, including the exact final merged binary. Private DeepSeek
sidecars used distinct ports. No conformance run used Service credentials or
submitted a model prompt. Mac probe files were removed after retaining receipts.

The packaged Codex restored a 31,200,735-byte synthetic Unicode rollout in 2.213
seconds cold and 2.131 seconds warm. Both restores preserved the native
identity, sent `excludeTurns`, launched the exact configured native executable
and returned 11,529 ACP bytes without replaying history or creating a new turn.

The Zed server build passed 50 source-owned native tests. Signed temporary
installation/drain, native synchronization and nonempty navigation gates passed.
The final Controller/Machine/native pair passed all 34 v14 connected checks in
293.97 seconds, including real authenticated installation and normal-timeout
lost-reply cases; fixture cleanup passed. These results do not claim physical
device acceptance or production language-server semantics from the test LSP.

Actual active, next-transaction recovery and cold roles were bound to the host
profiles and bootstrap closure. Final immutable readers passed 174 Controller
and 84 Machine journal checks. Sixteen full-Catalog reads passed before and
after activation. Actual Controller host policy and Machine writer-policy
preflights passed. No host policy, publisher key or credential was rotated.

## Published and installed Plugins

Each release was independently verified, published to both Catalog roots and
read back through its digest-bound HTTPS URLs: 24 unique Agent artifacts and
three Zed artifacts. The live Catalog reports every exact version below as
`ready`. Exact embedded Provider coverage also passed.

| Plugin          | Version | Composite SHA-256                                                  |
| --------------- | ------- | ------------------------------------------------------------------ |
| claude-code     | 3.1.35  | `051263cb77807d62913ed0054116851778fd482d22cc605eb965146a49ff5ebe` |
| claude-deepseek | 3.1.25  | `87a1dca8b1f2f0a76dd90d9496b078621b96bbab4fd562f10908736ab798cafe` |
| codex           | 3.1.29  | `65949bf8baec392e37090aeab435b718c8d4095e1939a9cdbdeb21c570d1f923` |
| codex-deepseek  | 3.1.25  | `23d227063ff05fa6b91e6b68b48010116eaa694ed9cefd54106916ee14891dba` |
| gemini          | 3.1.25  | `aef62039b66c073533cb5c576afefdf8a83032bb34e4bf0d65ef73a62e358973` |
| grok            | 3.1.26  | `5ae37ee04d547a075cbacc8fb36a90ee3bf632497410e199a92bd603dd78bd5d` |
| zed             | 1.20.2  | `d3bea276ec4d772e33915c20fb3abe7e35bc6acf6b10ba17cce2558a386ba498` |

Hawk installed all seven through the existing delegated Operator. Every
`hawk-<plugin>-<hyphenated-version>-converge` operation is `completed` with an
`applied` Machine receipt and the exact digest above. Final inventory agrees;
Hawk's convergence plan is empty. Victoria remains at 1.1.0. GitHub 0.1.0
remains published and ready; this upgrade did not add it to Machine membership.

## Component activation

The installed activator's public invocation is
`cowboy-release-activate <release>`, with `--maintenance` for the Machine lane.
Its `--machine` option is reserved for the internal root transaction.

| Lane       | Immutable release                                                       | Successful transaction             |
| ---------- | ----------------------------------------------------------------------- | ---------------------------------- |
| Web        | `/nix/store/xq2vba9p521xj7iir1fyfm88b8rhxc8k-cowboy-web-release`        | `1790767478496685299-ad3c076ce669` |
| Controller | `/nix/store/6jl584x0j8da275f1i2kzmvqnh2wz2b1-cowboy-controller-release` | `1790767520107877251-ad3c076ce669` |
| Machine    | `/nix/store/8pzsk6brw1jyjk8g00vs091gmjjp2v8z-cowboy-machine-release`    | `1790767552106094327-ad3c076ce669` |

All receipts are `succeeded/committed`, published, and identify source
`ad3c076c`. Controller PID changed from 438944 to 603952; resident Machine PID
changed from 1384210 to 605565 through its separate maintenance transaction.
Hawk is online at worker generation `worker-a740473b97661a21d1aa`. Its workspace
revision and identity hash remain unchanged.

Public HTTPS `/healthz`, `/version`, the SPA and `sw.js` all return 200. The SPA
version/ETag is `87a4277dc6f76b4d7442522c6f914b7c`; HTML and the service worker
are `no-store`, and the shipped service worker is 1770. Existing PWA clients
need a hard reload to load the new bundle. No iOS device build was shipped.

Seventeen original worker PIDs and start times are unchanged. One other unit,
`sess-1789954176241`, changed from PID 1297047 to 609858 at 19:27:22 CST. The
broker recorded that session as `Exited` before an explicit revive and then
selected the desired generation. This task sent no session stop or revive. There
are still 18 active worker units; this is not a claim that all 18 original
processes remained unchanged. No kernel OOM event was observed in that window.

## Remote Machine prerequisites still outstanding

Falcon's normal requests returned `409/plugin_sdk_unsupported` for all six Agent
upgrades: its active `db32ce3c` Machine supports SDK 1.8.1, while these packages
require 1.9.0. Its actual cold bootstrap remains `0c854389` and lacks
installation-journal admission support. The new immutable Machine was copied to
its store, but no component activation, host-policy change or NixOS release was
performed. A compatible host recovery baseline and its actual reader gates must
precede that Machine upgrade. Copying an artifact is not activation.

After Hawk converged, a separate Hawk-first run checked macbook-air. Five
requests returned the same SDK refusal; Claude DeepSeek returned "the Machine
did not report an observable installation target." Both remote hosts actually
enable the admission flag, so the older hostname-based documentation was
corrected. Their new operation identities were refused before installation; no
new identity was substituted for a retry. Their inventories remain on the
previous releases. Actual macOS execution of a Plugin payload does not upgrade
the resident Machine or establish its recovery baseline.

The final dry run records those 12 remote Agent upgrades and Falcon's Zed 1.20.0
to 1.20.2 upgrade as remaining. The latter also requires SDK 1.9 and was not
dispatched after the Machine incompatibility was established. These host
prerequisites, the retained major-version migrations above and the iOS linker
blocker are explicitly outside the completed production adoption.

The evidence root contains `evidence-index.json` with hashes of the final gates,
publication receipts, seven applied installation receipts, component and worker
observations, public HTTP results and remaining fleet plan. The pre-existing
unrelated Stormbird repair unit remains the only failed system unit.
