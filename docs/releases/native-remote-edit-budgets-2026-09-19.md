# Native remote-edit admission release — 2026-09-19

Zed Plugin/private adapter **1.20.0**, selecting private server **1.6.0**, is
verified, signed and published as `ready` for Linux x86_64. At the recorded
post-publication observation, **Hawk still has Zed 1.19.0 installed**. This task
dispatched no production Plugin installation or Controller/Web/Machine/host
activation. This is not whole-refactor completion or native-owner recovery.

Exact implementation/build/sign/publication source:
`b5be5b752ca9612a311a043a5d4690717a16c4f9`. Subsequent integration merges preserve
that commit rather than rewriting the provenance of the published artifacts.
The [design](../plugin-native-remote-edits.md) describes the admission contract.

## Scope and fixes

The local private server accepts `UpdateBuffer` edit/undo only for an existing
buffer, its original sharing peer and the remote-server project. Unknown or
expired IDs cannot allocate waiting-operation entries. Raw operations are
checked before upstream narrowing/dense-clock allocation: 128 operations,
4 MiB aggregate inserted text, 16,384 parts and 256 replica slots. This is after
protobuf transport decoding; it does not bound that preceding decoder.

Existing and prospective history share the sync/reload limits: 8 MiB retained
base/inserted text, 4,096 operations, 16,384 parts and 4 MiB visible text. A
detached native branch validates the whole causal/Unicode/history result in
the same GPUI turn, before publishing any operation. Exact duplicates consume
no history; conflicting identities, missing predecessors, invalid UTF-8
FullOffsets (including tombstones), overflow and read-only targets refuse the
whole batch. There is no prefix mutation, new deferred queue, history pruning,
implicit replay, filesystem effect or recovery grant.

The real-RPC fixture initially expected the sender's native mirror to receive
its own edit. Native remote edits are not echoed to that sender. The corrected
fixture uses independent exact-version plaintext navigation queries and
stale-vector refusal; native GPUI tests verify complete text and history.
An unchanged sender mirror alone is not native no-mutation evidence.

Connected acceptance also exposed two independent core/test defects:

- A local Operator socket owner could leave its flock held by an inherited or
  duplicated open-file description after teardown. The new RAII owner explicitly
  unlocks on drop, including failed-bind cleanup. A deterministic regression
  first reproduced the old rebind failure, then verified reacquisition while
  the old duplicate survives and that dropping it cannot unlock a successor.
  Existing live revocation and inode-matched socket cleanup remain intact.
- The Operator conformance relay still assumed Machine protocol 19. It now
  retains the actual Hello range and accepts only negotiated **19–21** within
  that range, clearing negotiation state on a fresh challenge. Missing Hello,
  out-of-range, old 18 and unknown 22 refuse. This changes the test relay and
  its receipt, not production protocol negotiation or authentication.

Upstream is still `aaf5f57dd36c41cf2ed49b13bcb091d52d5aef45`; third-party pins
are unchanged. Zed component release `3.11.0`, SDK `1.8.1`, Code component
`1.2.0`, payload schema 2, outer release schema 1, adapter API 1 and Machine
protocol 21 are unchanged. The repository-wide component graph is separately
`3.12.0`. Ordinary Zed is untouched.

## Exact published artifacts

| Artifact | SHA-256 |
| --- | --- |
| Adapter ELF | `19ab0d291ccd51d9049d17eb88659effd3ee60634ec2365e20e77d8aff8b14ec` |
| Private server ELF | `f0d700155fb4b6fc1b25ec96d33d53ecd31e2278cab93f783def78003826930c` |
| Package | `d723bf69a931181b30ad82a328767cec8e6626acf17bea64d7fbf2c3e58684c9` |
| Composite artifact | `b000481cf7548af7598bad0498141daca1d1e422efa2ac1dd90d8cb00830b65f` |
| Contract fingerprint | `b317168255b68ce092c035cb90648986984c3de06b24da26e3a7bcc3c92ab6a9` |

Immutable native outputs:

- `/nix/store/ng88l6jhmpnk07rxjwmz2s00l6llnwk0-cowboy-zed-adapter-x86_64-unknown-linux-musl-1.20.0`
- `/nix/store/i9yxq38r7pzhdkmp4bq8sypihaqlnxpq-cowboy-zed-server-x86_64-unknown-linux-musl-1.6.0`

The package-owned builder verifies declared versions, static portable ELF
requirements and copied-byte probes in disposable credential-free homes.
The existing `cowboy-first-party` publisher signed the release. The independently
selected immutable SDK verifier at
`/nix/store/bdjawvlvkv9w0hfbq8aljwb9rsk5da8c-cowboy-plugin-pack-1.8.1/bin/cowboy-plugin-pack`
verified the configured trusted public key. No publisher/key rotation or
Service login was needed.

## Accepted gates and supplied-role boundaries

- Complete `just check-compact`: main Rust **1,489 passed / 34 explicitly
  ignored**, standalone Machine **375 / 4 ignored**, core Code adapter **26**,
  private adapter **126 / 2 ignored**, frontend **1,816**, all **18** isolated
  PostgreSQL tests, and the SDK/package/component/format/lint/type/dependency/
  native-shell/website/composition/optimized-build checks. Independent
  `just plugin-check` also passed.
- Native source-owned Nix derivation: **57 tests** (3 filesystem, 4 LSP,
  50 project), including nine new remote-edit groups. The final clean runtime
  builder selects that exact tested output; a cached build is not another run
  of the native suite.
- Exact immutable pair: native sync/navigation conformance **6.01 s**;
  temporary signed install/uninstall/drain/reactivation lifecycle **11.71 s**,
  including nonempty navigation. Neither is a live Hawk installation.
- Connected v6: **19 checks** with candidate Controller `b5be5b75` and original
  Machine `3f82f19c` (**170.90 s**); another **19 checks** with observed Controller
  `15970ec0` and independently updated Machine `7269199e` (**171.43 s**).
  Both use the exact new native pair, disposable actual authentication,
  enrollment and HTTP lifecycle. Both record `stage=complete`, `failure=null`,
  `cleanup=true`, `accepted=true`. The second pair was captured before the
  later Controller `16882597` activation; it is not a connected run of that
  later Controller.
- Local Operator: **9 checks** against candidate Controller `b5be5b75` with
  each of those two Machines (**8.59 s**, **7.62 s**), recording protocol 21.
  Default denial, private-UID/TCP separation, one-effect dispatch, revocation
  and saved-ID/restart no-replay pass.
- Firefox 151.0.1: **24 buffer-owner checks**, fixture SHA-256
  `4e7a11c5ac7d212fa57370ad912bca000d784143febfa4d683eece9bd7b2da27`.
  These use synthetic browser evidence, not physical-device acceptance.
- Immutable source boundary:
  `/nix/store/0h4kfv40anmgy6gh1b7gir6pgl8j0p5m-cowboy-source-boundary`.

Candidate Controller:
`/nix/store/rzh47v5yh7kzfx5z0vrfd62pnmx571jq-cowboy-controller-release`.
Original Machine:
`/nix/store/03li1x7fh85ga3ycqiwwhz47l89af442-cowboy-machine-release`.
Observed Controller `15970ec0`:
`/nix/store/rlg49dn64q1588mhn2kb4i2ws2hdqqhc-cowboy-controller-release`.
Updated Machine:
`/nix/store/rg1266170l053dkzb292b2hrlxq8xhyh-cowboy-machine-release`.

Subsequent source integration preserves the published commit and merges remote
runtime-refusal and offline-replay changes through `bc28e8b0`. At integrated
source `0578b65710263cfc8f5cee64d315766264ee0f03`, the affected composition,
format/lint, strict types, feature slices, tests, isolated PostgreSQL, optimized
builds, website and IndexedDB-harness checks pass. This rerun has **1,497 main
Rust** and **1,818 Web** passing tests, with the same 375 standalone Machine,
26 core Code, 126 private-adapter and 18 PostgreSQL counts. No Plugin/runtime/
component inputs changed in those merges. This is separate source-integration
evidence, not a rebuild or resigning of the published `b5be5b75` artifacts.

## Publication and actual Catalog readers

Canonical `just plugin-publish` verified and published immutable bytes at
**2026-09-19T04:45:31.235Z** in `/var/lib/cowboy/plugin-catalog`, receipt:
`receipts/zed-1.20.0-b000481cf7548af7598bad0498141daca1d1e422efa2ac1dd90d8cb00830b65f.json`.
All three public HTTPS downloads match the signed hashes, immutable cache
headers and digest ETags. Delegated Operator refresh succeeds and the Catalog
advertises the exact version, kind, component release, package/composite
digests, fingerprint and Linux x86_64 platform as `ready`. Anonymous
`/api/plugins` remains HTTP 401; no browser credentials were copied.

Each of the staged, published and settled snapshots covers the complete
**89-release** Catalog from both configured roots. Each passes two cold process
reads per Controller role (active, next-transaction recovery, actual cold,
candidate): **8 reads and 4 actual-Service host-policy preflights per snapshot**.
Active/recovery initially use `15970ec0`; settled reads at
`2026-09-19T04:53:14.485Z` use `16882597` at
`/nix/store/idh4ms1kbcp48s0c082zbb39s5rca16n-cowboy-controller-release`.
The actual cold Controller remains
`/nix/store/hn2zd44ngda15pz6ki1qdjdw6c7ifmfh-cowboy-controller-release`.
The recovery role means the current profile a new transaction would capture,
not the old receipt's historical `previousRelease`.

Managed telemetry writer/background policies are `unconfigured`; legacy
selection is `not_checked`. No Catalog history or host policy changed. These
Catalog/configuration checks are not a new journal-reader matrix, production
login, native-generation installation or post-effect recovery.

The final handoff floor at `2026-09-19T05:09:44.031Z` repeats all 8 Catalog reads
and 4 host-policy preflights against the same 89 releases, with active/recovery
Controller `bc28e8b0` and unchanged cold/candidate roles. All pass. This is
separate from the earlier settled snapshot and still not connected native
acceptance of that newer Controller.

## Controller activation is held

The exact source Agent-publication gate fails against the primary Catalog
`/var/lib/cowboy/plugins/catalog`. Claude Code `3.1.28` is covered, but the
following exact signed versions are absent:

| Agent Plugin | Missing publication |
| --- | --- |
| Claude Code · DeepSeek | `claude-deepseek@3.1.19` |
| Codex | `codex@3.1.22` |
| Codex · DeepSeek | `codex-deepseek@3.1.19` |
| Gemini | `gemini@3.1.19` |
| Grok | `grok@3.1.20` |

The legacy Catalog alone lacks all six; neither root supplies the five missing
versions. These manifests came from integrated main, not this Zed change.
No gate was waived, manifest downgraded or unrelated Agent release published
to bypass it. **No Controller activation was dispatched**, so the Operator
lock repair is source-verified, not deployed by this task. A later activation
requires those independent release gates and a fresh build from integrated
main, not this now-older immutable candidate.
The final handoff recheck against the primary Catalog fails for the same five
versions; its separate `provider-coverage-handoff.log` is retained.

## Concurrent production observations and remaining work

The strict post-publication continuity audit **failed** because another task
changed production components. Its failure is retained, not relabeled as a
successful continuity check. Read-only reconciliation records:

- Machine transaction `1789790734170366602-7269199ebc51`, committed at
  `2026-09-19T04:05:45.682031946Z`, revision
  `7269199ebc516a6a9c675d1cab7367801cca146f`, `maintenance=true`, generation
  `worker-3a3c8b33f96774b1981e`.
- Controller transaction `1789793477743056449-168825973bce`, committed at
  `2026-09-19T04:51:46.093842136Z`, revision
  `168825973bce69233de3f28cd4374e172c2c515b`, `maintenance=false`.
- At `2026-09-19T04:53:06.154Z`, only **8 of the original 18 ACP worker
  PID/start/executable identities** remain (16 ACP workers are then present).
  Causes for missing/replaced workers are not established by this observation.
  There is no claim that all original processes or native owners survived.

Neither transaction was issued by this task. Web profile/receipt, host closure,
three Victoria process identities, Plugin inventory and Zed operation history
remain equal to this task's baseline in that observation. Local/public health,
SPA version and Machine health checks pass. The observed Web profile is
`/nix/store/qxj58i675sp574psbw2qk8ch3nqqgx0w-cowboy-web-release`; host closure is
`/nix/store/g612grcjk53982zsl0sl3yp2v84filgx-nixos-system-hawk-26.05.20260731.5b4f72e`.

Hawk Zed remains `1.19.0`, composite
`04bec5bfe03fbc6aeabe1701102dbbe24d3840007d7bbdbf1087bee83664279a`, installation
revision `installation-3b4be39a53e6574ecf5aa314c42fbee764eb5175b91ad2632099e7d70803bdc6`.
Availability does not update an existing native process. A live upgrade needs
the separate exact-release installation step; no reply to the optional upgrade
confirmation had been received at this publication handoff.

A later bounded inspection at `2026-09-19T05:08:37.852Z` again observes that exact
Zed installation and public health 200. The independently deployed Controller
has since advanced to `bc28e8b00fb602809737915aba21ca65cf266473` at
`/nix/store/a64y42ivj4xbjf5bbavbganxmazzi868-cowboy-controller-release`, transaction
`1789793812913081116-bc28e8b00fb6`, committed at
`2026-09-19T04:57:17.064414921Z`. This later observation is not another connected
conformance run or full-process continuity audit.

This bounds one more writer. Remote-replica bootstrap, other native/LSP writers,
global retained bytes and snapshot counts, general background budgets,
independently authorized post-effect restoration and supported-device consumer
acceptance remain open, alongside the broader typed graph/state-compatibility
[completion exits](../plugin-refactor-completion.md). Forced fixture cleanup,
successful Catalog reads and native refusal do not close those exits.

## Evidence

Private evidence directory: `/tmp/cowboy-native-remote-edits.cInwrV`.
`evidence.json` binds the exact release, native outputs, all accepted gates,
failed-then-fixed regressions, publication, reader floors and the explicitly
failed continuity observation. Its SHA-256 is
`f8c45c450cde33aca8eb9169a19819aaefda58b1f3be90ab070deeffd7aed400`.
It contains no private key or raw Service environment.

| Evidence file | SHA-256 |
| --- | --- |
| `check-complete.log` | `a769cfe9f7f8b49a5b2090487caec1934c26954beec203bb62ef16deb119a316` |
| `native-build-2.log` | `9a83484f8891c48b6c1801e8c789cdea3742b108238fe3057027078309ce9f53` |
| `connected-accepted.json` | `c9ebab9ad6eb523ae4b50349eb1486eff6f36056a890432d1508c2520384964a` |
| `connected-current.json` | `3640f6c38c384b10085b21c50d9a60f96e8ce96fe2a8e9862b8a2334f68d0b08` |
| `operator-current.json` | `14a822dca3ecd83fcfc4092c285e23df15d4fb99df02b91f0677dc2de1a03f58` |
| `signed-release.json` | `7876a44235e7f2b466a743b5bc048c89f26d839f444aca9a3295816144290675` |
| `settled-floor.json` | `a373dc358b8a6a3f30d83197839eb1178d222d2387958ac9e68f8c3db4108b48` |
| `public-downloads.json` | `48c892b3f3de8515cc0e831e6bbe26367e241418fde0bcac231d5f87783ad028` |
| `audit-published.log` (failed) | `a75e505fa8e6168b309e1111364e8e900ac24803a0772b1e2912eeb1a602eb9c` |
| `production-observation-final.json` | `eda4e383f4da47fd8b168c4e1da68285b28a5be46e723ee09c088cb183be5362` |
| `provider-coverage-accepted.log` (failed despite filename) | `d0bf4d92badd4e2817797aca3b68400a80182718a1ce142470d2fd6f35057ba9` |
| `provider-coverage-handoff.log` (failed) | `0b94d6dbf2b0d470c1a1f2e6433421caef535454af9c1ec65a88dc6520a44ad7` |
| `handoff-installation.json` | `418b86bc549c064fc95fde91b89f9e79ca9ce7d4d56f77b312a05ea6c30be6f6` |
| `handoff-floor.json` | `558841f288aa0a6e8affb712e7c2ff16c6608175278972a6e7c4d65cf3ac66ee` |
| `integrated-check.log` | `7c331efc0b9b6b2a4a3ab1317bc0797a90a376c71a13e997a087f7c570d3af3c` |
