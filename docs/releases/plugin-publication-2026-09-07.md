# Plugin publication — 2026-09-07

## Result and authority

The user requested publication and explicitly waived historical-version
coexistence as a prerequisite: “请发布，不用考虑旧版本，全升级了”. The
repository-owned `release-cowboy-plugin` workflow published all seven current
candidates at **2026-09-07 10:38:29–10:38:37 UTC**. This is a completed
immutable Catalog publication. The initial publication did **not** activate
components; the subsequent user-confirmed maintenance is recorded below.

The Catalog `/var/lib/cowboy/plugin-catalog` now contains **50 signed
releases**. All **230 original files / 43 historical releases** retained their
exact bytes. Historical failure receipts remain unchanged; no failed coexistence
test was relabeled as passing, no release gate was weakened in code, and no old
version was deleted. Current-version integrity, native-platform and worker
acceptance were required and passed.

| Plugin          | Version | Composite artifact digest (SHA-256)                                |
| --------------- | ------- | ------------------------------------------------------------------ |
| claude-code     | 3.1.14  | `7033807a6d08554cf9b12706c03a3da3494c70ac62d4b242810cf116fb70806c` |
| claude-deepseek | 3.1.14  | `ca0d5ddb07aa4a01bbd09687660f513e08ff3ec6da394def7ada1cf1cc724226` |
| codex           | 3.1.14  | `293ae7ce9aeee75a1dbb88557d66466c4e9b35786988bb324c8d854d5d43fb55` |
| codex-deepseek  | 3.1.14  | `1d443d4c5133b6dcf7f5636c22e12228feb698afbe64d64843d56c7d1bb5fc1a` |
| gemini          | 3.1.14  | `ebb50a5eed286f3a3cb1861fe687544922c93e966221549e54632953e6eacb88` |
| grok            | 3.1.14  | `2be9a5aadd6aa946b5e6700c587234987c92c83618735a8d9ce3709ce388720c` |
| zed             | 1.2.2   | `b5695b999b6be2c396bb22c74079a8c217b064874dd04f53f1eeedb838b2e699` |

Password/Passkey 1.0.0 and Cardea 1.2.0 were already published and were not
republished. The configured existing Ed25519 publisher and independently trusted
public key matched fingerprint
`SHA256:a/VJzmHD/94vMVQMNZTktSR9P3apkkKnnWzXrtn02hg`. No key or trust identity
was created or rotated.

## Verification

- Fresh Cowboy `origin/main` at `983de476b3b9b9e148a69a2585ca57b0bb5887d3` was
  integrated without rewriting task history. Incoming authentication recovery
  behavior was retained alongside the Plugin UI. Service-worker cache version is
  1635, with both exact cache-version tests updated.
- The pinned-shell complete `just check-compact` gate passed after integration,
  including Rust, Web, Plugin schemas/runtimes/auth, isolated PostgreSQL tests,
  and release builds. The final ancestry merge changes documentation only;
  Controller/worker executable hashes remain identical to the accepted build.
- All six Agents passed the exact Linux x86_64 **real worker** initialize,
  session-new, sidecar (where present), stop and descendant-drain checks. Each
  receipt explicitly leaves old/new coexistence `not_checked`.
- All six Agents passed fresh **actual macOS aarch64** probes, covering eight
  unique runtime artifacts. Probes used private temporary homes, no Service or
  Provider credentials, and no inference prompt. The remote test directory was
  removed after preserving all six receipts locally.
- Zed's exact Linux adapter/server passed the owned static executable audits and
  real signed installation, lease/drain and reactivation test. This was a
  disposable fixture, not installation on a registered Machine.
- The owned actual-reader conformance gate passed all seven exact publication
  preflights. Complete staged and production Catalog audits independently
  verified all 50 signatures and artifact sets. Two cold reads with the active
  bridge retained its original 34 supported identities; the full successor read
  all 50. Exact temporary Host policy pins passed successor preflight.
- All **25 unique public package/runtime URLs** were fetched without credentials
  and verified against their signed SHA-256, immutable cache headers and ETags.
  `just provider-release-coverage /var/lib/cowboy/plugin-catalog` passed all six
  embedded Agent versions.
- Dependency audit found no integrity mismatches. Source/runtime lock pins were
  retained; newer untested upstream Claude, Gemini and Grok versions were not
  folded into this release merely to make an upstream-version number current.

Durable publication receipts are in the Catalog's `receipts/` directory, using
`<id>-<version>-<composite-digest>.json`. Local detailed evidence is under
`dist/provider-runtime-cache/full-rollout-1d0a5064/`: `baseline.json`,
`reader-conformance.json`, `audit-stage.json`, `audit-production.json`,
`urls.json`, and `mac-receipts/`. Current Linux worker receipts are under
`dist/provider-runtime-cache/full-upgrade-23efaa8f/<id>-worker.json`. These
local evidence directories are ignored artifacts, not portable paths.

## Components built at publication time

All three Hawk Nix outputs were built from clean committed source
`1d0a5064b794f2384923ae7c8a558a841a0ae008`, which descends from fresh main and
the active Controller and both Machine provenance floors:

- Controller:
  `/nix/store/hcdzb1vpvb7mm61pawqywgv36m4a12iw-cowboy-controller-release`.
- Web: `/nix/store/j8i0nffh4qlzih1fj1qlh8cww9zkxsa0-cowboy-web-release`.
- Machine: `/nix/store/9gdn5kdfnz80b00vaxkjcfsz1x03n2xg-cowboy-machine-release`;
  worker generation `worker-4718f2ebbba9e4069713`.

Controller SHA-256 is
`adb17f860bb0a5ae97b4a3c939d092a5aea04e0be06d3ad167535ad95f9ae93d`; worker
SHA-256 is `60c9dbf781b47a2ea055fdd542fb556922c5c48d45ee47f30238b4503cae9fa9`.
The source commit is published on `cowboy/sess-1788279284752`. Falcon
independently completed the same exact Machine output from its clean task
worktree `/home/draven/worktrees/cowboy/plugin-full-rollout-20260907`. No stable
checkout was edited or used as a deployment source.

## Initial live-transition blockers

This section records the boundary before the user's subsequent confirmation; the
maintenance results below supersede its unactivated component state.

No component was activated, Product login configuration changed, Plugin
installed on Hawk/Falcon, session rebound, active turn stopped, or account /
Provider credential changed by this publication. Hawk's Controller PID remained
4158242 with zero restarts; health remained good and both Machines stayed online
on their original worker generations (`worker-a4ad441efe0461691687` on Hawk,
`worker-e11838bbb2e6e27902da` on Falcon).

The full successor's **read-only preflight with actual Hawk Service paths and
authentication/database configuration** rejects the current policy:

```text
bootstrap requires one WebAuthn storage host; configure an exact host selection for a published storage Plugin
```

The current NixOS generator still selects hostless Cardea 1.1.0 and supplies no
`COWBOY_PLUGIN_HOST_CONFIG`. Editing its generated runtime JSON would not be a
durable fix. First Host activation requires a separately reviewed NixOS policy
transition with exact Password/Passkey and Cardea selections and an accepted
storage/authority recovery plan. The installed Columbus activator `a872284`
restores only the previous component profile on failure; the current legacy
bridge refuses post-Host authority. That is not a valid post-migration recovery
strategy. Never delete authority markers or rewrite applied migrations to make
the bridge start.

A freshly fetched, clean Columbus task worktree was prepared at
`/home/draven/worktrees/columbus/cowboy-plugin-full-rollout-20260907`, based on
active/fresh-main `a8722843904e17462add70af0f9743d2d4815546`. It has no policy
changes or deployment. Component publication does not implicitly authorize a
separate NixOS service-policy maintenance switch.

Machine Plugin installation additionally requires authorized Cowboy Product API
credentials. `/api/plugins` returns 401 without them. A temporary token in a
Hawk file with mode 0600 was requested from the user; no token was supplied,
borrowed or minted. Installation and authenticated refresh/UI availability
remain unverified. Raw artifact availability is not live Catalog availability:
the active bridge intentionally cannot offer the newly published formats.

Actual upstream native-history resume and the first queued prompt observing
completed configuration restoration remain independent follow-up acceptance. The
publication waiver does not constitute either result.

## Confirmed Host maintenance and component rollout

The user subsequently confirmed Hawk's authentication/Passkey migration
maintenance. No credential values, account mappings, signing keys, session
lifetimes, enrollment tokens or authentication enforcement were changed.

The recovery implementation was independently reviewed. Columbus changes passed
`nix develop -c just verify`, including all Go tests/vet and three
Plugin-storage backup tests:

- `f334336e90228cec7be5076a9c7b661cf58ec7a4` added explicit immutable Controller
  `--recovery-release` support, exact bootstrap Host pins and Plugin-state
  backups. Recovery identity must satisfy fresh main, active provenance and
  candidate ancestry. Missing or mismatched recovery fails closed, never
  restoring a pre-Host executable against post-Host state. Interrupted recovery
  retains truthful provenance; independent Web-root recovery is still attempted
  if restoring its Nix profile fails. Both review findings received regression
  tests and were resolved before activation.
- `c9f16a0b61dad07ed35e4fa585a982753d5ffd86` enabled `catalog_only`, selected
  Cardea 1.2.0, and removed the legacy Controller OIDC environment entry. Hawk's
  active system is
  `/nix/store/1pyvm60m5jal2d8w7wajs7g3ja9pbpnl-nixos-system-hawk-26.05.20260731.5b4f72e`.
  Both host-policy transactions retained the live Controller/Machine/Zed PIDs.
  Component restarts were separate explicit transactions.
- `3e0f0100` exposed the existing versioned transaction app through
  `cowboy-machine-activate <release> candidate`, for a target whose installed
  deployment-health protocol is obsolete. It requires a clean committed isolated
  Columbus worktree containing fresh main, preserves the same lock, journal and
  provenance checks, and does not switch NixOS or bypass auth. Invalid
  transaction selections and dirty-source rejection were checked.

All three Columbus commits were pushed to `origin/main` without rewriting or
editing the stable checkout. The actual accepted Controller binary and the
explicit recovery binary were exercised together in a private loopback-only
fixture: core user/admin Passkey import, abrupt process loss, retained new and
updated credentials, one-shot import, legacy downgrade rejection, catalog-only
restart and compatible recovery all passed. This is not a real login/device
ceremony or proof against every application defect shared by both binaries.

Hawk's actual generated service configuration passed `--check-plugin-hosts` with
both immutable binaries before each transition. The Controller release from
clean `7118cf335b0176524afbf9493e04c710b4c64bfc` is
`/nix/store/y90siyv9qzl3i1a2ih9g3gmdir0rnmab-cowboy-controller-release`; its
executable hash equals the accepted 1d0a5064 build above. The first Controller
transaction succeeded at **11:53:51 UTC**, with the 1d0a5064 closure as its
explicit compatible recovery. The final catalog-only transaction succeeded at
**11:59:04 UTC**, with the already Host-capable 7118cf33 release as its
predecessor. Neither transaction tried to downgrade Plugin storage.

The live Passkey namespace is
`/var/lib/cowboy/plugins/live/passkey/state/db.sqlite`. Its imported rows match
the original core rows by full-row SHA-256; the core rows remain unchanged,
SQLite `quick_check` passes, and the import ledger contains exactly one entry.
Public `/api/auth/status` reports Password 1.0.0, Passkey 1.0.0 and Cardea 1.2.0
Hosts. The catalog-only and per-Host authority markers are retained.

The Web component succeeded at **11:56:32 UTC**:
`/nix/store/a8d5gyfg6h8l5vnidncahccy579xw2f3-cowboy-web-release`. The public
`/version` is `fe9bf606cf496afbbd5f396b14eb5a20`, SW cache version is 1635, and
both HTML/SW responses use `Cache-Control: no-store`. Installed PWAs still need
a hard reload; reconnecting the WebSocket does not replace old JS.

Hawk's Machine component succeeded at **11:55:42 UTC**, using the exact
`9gdn5kdfnz80b00vaxkjcfsz1x03n2xg` release above. Its live generation is
`worker-4718f2ebbba9e4069713`.

Falcon's first attempt proved the new generation could connect, but its
installed `744a6f5` transaction still probed `/api/machines`, which correctly
returned 401. It restored the previous Machine profile, then recorded a failed
recovery-health check for the same reason. These failed receipts are retained.
The retry through the owned current transaction succeeded at **12:11:33 UTC**.
It recovered the interrupted journal, preserved both the failed and
`recovered-rolled-back` receipts, then activated the exact 1d0a5064 Machine
release. Both Machines now report `worker-4718f2ebbba9e4069713`, connected and
online. Falcon's NixOS closure was unchanged; until its installed deployment
tool is updated in a separate host release, use the documented `candidate`
transaction source for subsequent Machine releases.

Backups retain the original core/artifact pair and now include individually
consistent Plugin SQLite snapshots plus one-way authority. Important sets are
`20260907T115254Z` (before import), `20260907T115832Z` (after import), and
`20260907T115942Z` (after catalog-only), under `/var/backup/cowboy/` with mode
0600. Each has `cowboy-<stamp>.sqlite3`, `artifacts-<stamp>.tar.zst`, and
`plugins-<stamp>.tar.zst`. The final Plugin archive was restored into private
scratch storage: integrity, import ledger, row counts, authority and standalone
non-WAL database checks passed. Live storage was not restored or rewritten.
These snapshots are not one cross-database transaction; a coordinated restore
still requires quiescing Cowboy and retaining a compatible Host policy.

Detailed ignored evidence lives in
`dist/provider-runtime-cache/host-cutover-20260907/`, including actual-binary
`migration-recovery.json`, both policy preflights, and private hash/count-only
`live-before-import.json`, `live-after-bootstrap.json`, and
`live-after-catalog-only.json`. The final `live-release.json` independently
checks all four successful component receipts, matching Hawk profiles, both
Machine generations, public authentication Host versions and exact HTML/SW
bytes/cache headers; unauthenticated `/api/plugins` still correctly returns 401.
Root-owned component receipts remain under
`/var/lib/<machine>-component-deployments/`; NixOS receipts remain under
`/var/lib/hawk-deployments/`.

## Remaining acceptance

Registered-Machine Plugin slot upgrades and authenticated Catalog/UI checks
still require a user-supplied Cowboy Product API token file (mode 0600 on Hawk).
None was supplied, borrowed or minted. Host/component rollout is not proof that
every registered Machine Plugin slot is upgraded. Real Passkey/OIDC login,
upstream native-history resume and first-queued-prompt configuration restoration
also remain unclaimed.
