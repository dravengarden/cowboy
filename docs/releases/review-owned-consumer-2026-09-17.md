# Content-owned Review source delivery — 2026-09-17

**Published on main and activated on Controller/Web.** The
[ordinary source consumer](../plugin-review-owned-consumer.md) now selects one
core buffer owner for language, hover, Outline and explicit synchronization
preparation. Complete displayed LF text supplies the conditional-read identity;
late file, manifest and annotation replies cannot attach to replacement text.
There is no legacy fallback after owned acquisition, automatic reload, repeated
Open, or implicit Apply/retirement. **Check Code** can observe an uncertain
initial Open by its original ID without repeating acquisition.

The active protocol-19 Machine still selects the pre-cutover route. This is not
production activation of owned Review, a Machine/Plugin upgrade, owned
navigation/diff integration, recovery or completion of the refactor.

## Immutable inputs

Clean implementation source: `032a5d42d870c0b7cc1f95c8c0a217f7258e0349`. The
implementation was rebased onto remote `780f7e0c`, retaining the independent
Composer/WeType changes and documentation. No native implementation, SDK,
Plugin/component version, dependency lock, durable format or host policy
changed.

- Controller:
  `/nix/store/68z30k2vf3rbjjq9cyk6nzphks0pxzah-cowboy-controller-release`.
- Web: `/nix/store/38xmd18jd4z95c9wasfw99xwvai2yddr-cowboy-web-release`.
- Web assets: `/nix/store/6b2cmv4xjwz92wb7wbqz16irkg4210rc-cowboy-web-0.1.0`.
- SPA version: `d5902800cd931e7484893326fd22ace8`; service worker:
  `cowboy-v1707`.

The documentation follow-up records these bytes; it does not require another
application activation.

## Gates

The final clean source passed the pinned-shell
`env RUST_TEST_THREADS=4 just check-compact`, including formatting, strict
Clippy, Web type/lint checks, dependency audits, component/Plugin contracts,
composition conformance, isolated PostgreSQL and release builds:

| Suite               | Passed |
| ------------------- | -----: |
| Main Rust           |  1,411 |
| Standalone Machine  |    333 |
| Core Code adapter   |     26 |
| Private Zed adapter |     75 |
| Web                 |  1,691 |
| Isolated PostgreSQL |     18 |

Main/Machine/private-Zed retain 32/two/two explicitly ignored tests. The 14
focused Review lifecycle cases include bounded serialized reads, cancellation,
failed/pending initial Open observation and source-abandonment fencing. Existing
dependency-policy, Web lint and large-chunk warnings remain visible.

Both immutable release builds and `nix flake check` passed. The initial formal
Controller build exposed a missing `code-buffer-sync.fixture.json` in its
cropped source, inherited from the preceding browser slice. The fix includes
that fixture in Controller and asserts that it remains outside Machine source.
The failed build is not counted as acceptance; both final outputs were rebuilt.

All 44 real Firefox `151.0.1` cases passed from final committed source, using
fresh profiles and isolated loopback namespaces with no production credentials:

| Browser suite          | Cases | Fixture SHA-256                                                    |
| ---------------------- | ----: | ------------------------------------------------------------------ |
| Review source consumer |     6 | `7e650c3e82b7a3d3596c908b3b1b20f983d5961816209302c205a62ad2fe6538` |
| Buffer/content owners  |     8 | `b78386233c49f102b09ceb9ce5f8d945edc15af71d7bfcaebba2182826b2edd7` |
| Product context        |     6 | `671fc16594ce3c0f5483ad4f26081113a76bdfbd3dd33bdfa00288378bd106f3` |
| Buffer cleanup         |     7 | `f5066b98c54928278091188e23cb730b532ed98a46691187643a92300199a1bc` |
| Synchronization        |     8 | `9b92537f08d4ec4ee5d708f788bd64714362b32d7176bc1eef2e91a0550e690d` |
| Settings recovery      |     9 | `a1b1f6e79a93f987b545523117922137a95dfd2a60aabe05872418bf5659246f` |

The new suite mounts the actual hook, Outline and status surface with React
development StrictMode and WebCrypto. Its HTTP is synthetic, not full logged-in
Review or physical iOS/WebKit acceptance. The Settings link sends only the
existing core navigation event; preparing refresh never confirms it.

Two fresh [connected process](../plugin-code-connected-conformance.md) runs
passed all eleven groups with schema v3, `accepted=true`, `cleanup=true` and the
final source revision. The actual candidate Controller manifest must select
Owned. Both use the unchanged supplied protocol-20 Machine candidate
`/nix/store/d5s9s8l6m3wph6lmsblzd41rhqy3kysa-cowboy-machine-release` (source
`ef84a341e4ca97144be7353ce5fef5f5db2fabc8`) and the exact native pair:

| Native executable                                                                                                       | SHA-256                                                            |
| ----------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------ |
| `/nix/store/k67bc55f5yrd4k8nlzri62m0z6kjg5rs-cowboy-zed-adapter-x86_64-unknown-linux-musl-1.9.0/bin/cowboy-zed-adapter` | `b9b5c5da9bf47e54a31bf807f6ac21387a3e9ed6a895da89b551e80016bdc5f3` |
| `/nix/store/lywsfx0zmq03ml9rjlqhhqw9dmp0gdvr-cowboy-zed-server-x86_64-unknown-linux-musl-1.0.0/bin/cowboy-zed-server`   | `da41ec6baee1cbf714b809fcd912dd200be150c634cbe4a6ec6939e323da8131` |

These runs perform real disposable login/enrollment, signed Code installation,
uninstall and ownership/synchronization operations. Each loses one real Apply
reply and waits the unchanged 40-second timeout before original-ID Query;
neither replays Apply. Fixture teardown is not product rollback or recovery.
Plaintext results do not establish nonempty LSP or atomic diagnostic freshness.

The actual candidate, current/next-recovery Controller `c48b3d12`, and cold
Controller `869c269f` each read all 84 Catalog releases twice with identical
reports. Their actual Service `catalog_only` host and typed telemetry
configuration preflights also agree. Arguments/environment were passed in memory
only; no credentials, policy or installation changed. This config-only preflight
is separate from the connected fixture and production health checks.

## Activation and continuity

Implementation was pushed before either activation. The machine-owned
transactions both committed with `outcome=succeeded`, `phase=committed`,
`published=true`, without a maintenance or recovery override:

| Lane       | Transaction                        | Previous release                                                        |
| ---------- | ---------------------------------- | ----------------------------------------------------------------------- |
| Controller | `1789621214292096989-032a5d42d870` | `/nix/store/q61apwxgfz8w30a280xk1sx8x7cg785c-cowboy-controller-release` |
| Web        | `1789621261430190060-032a5d42d870` | `/nix/store/iv40f5jyz7d7xzg3ybzzjwh3fyn5y771-cowboy-web-release`        |

The **12:59:07–13:01:16 +08:00** window retained all **16 original worker** and
**four native Code** PID/start-time identities. Only Controller restarted;
Machine, Victoria, host/cold-recovery files, workspaces, installed Plugin
identities and failed-unit sets remained unchanged. Persistence was healthy with
zero dropped/failed batches in both observations. These finite observations do
not establish uninterrupted native generation replacement or recovery.

Machine is online on `worker-4208d4d141de95cf9feb`; installed Zed remains
`1.8.0` with generation
`sha256:56474a7197fb8ba30d401236e780a35f107a9e9e7a5ab9869445c0d53a425d20`. No
Machine activation or production Code installation was issued.

Local/public `/healthz` and `/version` passed. Index, admin, service worker and
both entry assets match the immutable Web output byte-for-byte. HTML/SW are
no-store and hashed assets immutable. Existing PWAs must adopt the new bundle;
server activation alone does not prove client reload or native consumer cutover.

## Evidence and remaining boundaries

Private scratch evidence: `/tmp/cowboy-review-owned-1MYx3NEb`, not a permanent
public artifact. Selected SHA-256:

- Complete gate:
  `a2589e8cd231557e9b9072c8306b2e680175fca5b84da43823c2ab35f4087865`.
- Browser gates:
  `a0fe178ac98aee2dfcbbd3848a7174c0fad89158dd4ea496a12c5e49c53d25c4`.
- Immutable builds:
  `521692e928651732a79c2ffb09a03b49571811ecf4bf44f89f1994c121bcc8ad`.
- Nix flake checks:
  `61e9764dcdcaaa5517f44e87ed045776d9c930c76bb47e3267c55f8e3dbbd00c`.
- Connected run 1:
  `e769b4eefaba74f23db7eff993dc864a4e2603252ea1540269e6b70c4045b419`.
- Connected run 2:
  `f8c5443a967433aa145d1c3269ecef8df13b4018ed107dd76aeb9726a14d1837`.
- Actual Catalog report:
  `4837dfe703de749ad78d8f97e7d652c3c0789ff56da2020a0454381476312c7d`.
- Actual host report:
  `cdd4615efec67ccdca9bc6e6caacd0bbc6dc00eb67744a2c47ff233eab34ff9c`.
- Rollout audit:
  `a77aa0c754cd684e37e1f073b4c62e2f99a7bd3c6e5a8c81f72e18ba814a547d`.
- HTTP receipt:
  `3d189fc9a59c3f9ed8973a8aed689a672625bad7c9476f683b61439c763806e8`.

Remaining Code exits include owned navigation destinations, diff coordinates,
the separately authorized protocol-20 Machine maintenance and signed Zed 1.9
publication/installation, actual native consumer and supported-device
acceptance, and abandoned-browser/restart/independent post-effect recovery.
General typed graph/site/state compatibility exits remain in the
[completion ledger](../plugin-refactor-completion.md). None is closed by this
conditional Controller/Web delivery.
