# Managed single-attempt telemetry rollout — 2026-09-12

Cowboy `51f799725bc8aef24deaaaa830bd0ef02716635a` is published to remote main
and accepted on Hawk as both Controller and Machine. It adds the staged
[protocol-16 managed OTLP attempt](../telemetry-managed-attempts.md). Both binding
writer gates remain closed; there is no production background-export activation
or automatic grant recovery. This is not P2 completion.

## Immutable source and gates

- Controller:
  `/nix/store/l5jskhg4xbsm46l70ksapjrmc8w8q51g-cowboy-controller-release`.
- Machine: `/nix/store/js8w2yjwsbgwzx9cjh285bzg5nvk5xdr-cowboy-machine-release`.
- Filtered-source check:
  `/nix/store/lgbmw8zqk6gs0g5s3xiiq0lgbfrf30q2-cowboy-source-boundary`.

Both release manifests record the exact clean source above. The pinned complete
`nix develop -c just check-compact` gate passed: 970 Rust library tests passed,
16 intentionally ignored; 1,371 Web tests and 14 isolated PostgreSQL tests passed.
Strict lint, dependency/feature-slice checks and release builds passed. Eleven
new managed-export tests cover exact typed requests/replies, Operator revocation,
original connection/deadline ownership, bounded lifecycle waits, durable evidence
tampering, signed installation, real JSON frames and all three official OTLP
client protobuf fixtures through actual HTTP. Partial success, 429, 503 and
redirects do not cause a second HTTP attempt.

## Owned component acceptance

The existing candidate activator came from clean isolated Columbus source
`42b3845233ebba35874fdc712c19a5400eb05876`. It fetched fresh main and used
the normal component lock, provenance checks, rollback journal and health gates.
There was no NixOS switch, direct profile override, Web activation, Provider
installation/authentication change or manual worker restart.

Controller transaction `1789213890073203208-51f799725bc8` committed successfully
at `2026-09-12T11:51:41.953396944Z` with `published=true`, `maintenance=false`.
Its predecessor is
`/nix/store/w55cm4nvnav1f58bgr8xgyvwxrqaiv7d-cowboy-controller-release`.
Actual Controller PID 1758990 started at `2026-09-12 19:51:30 CST`; its executable
resolves to `/nix/store/ijvf6mvwcba381vvrlqg2hi1pw4xqimi-cowboy-0.1.0/bin/cowboy`.

Machine transaction `1789213934295736475-51f799725bc8` committed successfully at
`2026-09-12T11:52:23.045060807Z` with `published=true`, `maintenance=true`.
Its predecessor is
`/nix/store/9zvsf5f507d7jd504a8ad82hq7yhlcpc-cowboy-machine-release`.
Actual Machine PID 1760647 started at `2026-09-12 19:52:14 CST`; its executable
is the wrapped binary within
`/nix/store/vmxzi8w33zpxvic8h10h4sj6f19fjj3h-cowboy-machine-0.1.0`.
The Machine authenticated with **protocol 16**. Both activation units ended
successfully; the machine-owned receipts remain under
`/var/lib/hawk-component-deployments/`.

## Continuity and closed admission

- The immutable and reported active worker generation both remain
  `worker-92b35f0665ec33ba60f6`. All **15** pre-activation active worker PID/start
  records were identical afterward. This does not claim old busy workers have
  all adopted that generation; no worker drain was forced by this release.
- Controller PID/start time did not change during Machine maintenance.
  `/healthz` returned `ok`, Machine presence was online, and workspace revision
  `f0d1093cec96b7f54144345bb1df5fb45acaf0e1` and workspace-ID hash
  `109fda30f8dad466162ab87d6c6b41fd8de2d20d149a548a2644602f594e23bd` were unchanged.
- Web remained at `/nix/store/f9jrl75r47cnsidlkdjg251zws30mif3-cowboy-web-0.1.0`.
  Public version/root ETag `c388e3a9f25b79e2562cca79e1d4ed12` and service-worker
  ETag `3bb30081331173c39260c1437c49e642` were unchanged. Both returned HTTP 200
  with `cache-control: no-store`.
- Controller startup reported `admission_enabled=false managed_namespace=false`.
  Machine `plugin-operations/telemetry-bindings-v1.json` remained absent as both
  file and symlink. The installation-incarnation journal's existing
  `writer_enabled=true` log is **not** telemetry binding admission.
- Private destination policy was neither read as release evidence nor changed.
  Signed Plugin/SDK bytes, native ABI and worker-generation inputs were unchanged.
  Existing local telemetry and explicitly configured legacy export were not
  migrated to this staged path. These receipts do not assert new live Victoria
  destination or background-export acceptance.

Before managed production activation, still require accepted live/rollback/cold
binding reader floors on both Sites, explicit writer/background authorization,
independently authorized interrupted-binding resolution, and cross-end restart
acceptance. Already emitted OTel data remains `NoRestore`; a missing response is
not permission to replay it.

Detailed gate, build and continuity observations are retained in
`/tmp/cowboy-managed-export.WzNxtI/acceptance.md` and adjacent logs/snapshots.
