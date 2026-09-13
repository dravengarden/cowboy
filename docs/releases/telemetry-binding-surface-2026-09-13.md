# Ordinary telemetry binding surface acceptance — 2026-09-13

Implementation: `9628f8722826047fc70ac68cd983aedf7d25f43d`.
Activated Controller/Web source:
`43ef8c92d29bcbee053cdc584a3f8a59f098bf0c`, published on main.

The [core binding surface](../telemetry-binding-surface.md) is available under
Settings → Info: discover exact installed targets, preview selection/revocation/
restoration, and inspect durable operation receipts. **Production confirmation
and managed export remain closed.** This is a tested confirmation boundary, not
P2 exit, automatic compensation or a new Plugin/SDK release.

## Gates and integration

The complete pinned `just check-compact` passed before main integration: 1,037
Rust library tests (19 intentionally ignored), 1,405 Web tests, 15 isolated
PostgreSQL tests, formatting, strict lint, dependency/feature/SDK/Plugin,
native-shell, site and production builds. Nine new HTTP tests include a real
signed Victoria installation, both restoration directions, exact target and
authority rejection, concurrency and an HTTP timeout before a real dropped
Machine ACK. Twenty-six type-checked Web telemetry-surface tests passed.

This task then rebased onto main's independent image-send fixes through
`3a9e4e8701e5bde2f3b396b84b920dcf5db9c85b`. No Rust, Cargo, Nix, fixture,
component or Plugin input changed during that integration. A stale source-string
test was updated to check the cached-timeline guard on **both** new echo paths;
the image-send implementation was retained. The complete integrated Web suite
passed **1,411** tests, plus typecheck, lint, site and production build. PWA
generation advanced to **v1673**, independently of the prior v1672 deployment.

The clean final Nix Controller build passed **812** library tests (15 ignored)
and its three binary tests. The new shared Rust/Web public fixture is explicitly
included in the Controller fileset. Both release manifests identify clean
source `43ef8c92`.

## Immutable releases and actual transactions

- Controller release:
  `/nix/store/w024allw47zf31clxvncyz941igvsykv-cowboy-controller-release`.
- Controller ELF:
  `/nix/store/f80i19c8zs6hsn8gkmql2y6v823247fm-cowboy-0.1.0/bin/cowboy`,
  SHA-256 `79b77c00cdb97c1cb47a995e7b1c5ca51d4dbd46970a0c456d78610fcc8410df`.
- Web release:
  `/nix/store/wb1fnr5wwlg16ls73m38dfvwd4ms3adl-cowboy-web-release`.
- Served Web root:
  `/nix/store/ca5ay0z29zpn2w7ba3m4yaa0r1szxmx0-cowboy-web-0.1.0`.

The installed machine-owned activator was invoked through the unchanged isolated
Columbus worktree `cowboy-controller-recovery-20260912` at
`de1f6c6504146de3633271fc223d3aaa3c449212`.

Controller transaction `1789272374452190852-43ef8c92d29b` completed at
`2026-09-13T04:06:38.090407021Z`; Web transaction
`1789272438434955635-43ef8c92d29b` completed at
`2026-09-13T04:07:18.472070429Z`. Both receipts say
`succeeded`, `committed`, `published=true`. Both root transaction units ended
inactive/dead, success, exit zero, with no incomplete journal.

The Controller's predecessor was
`/nix/store/6jih7xw4jrs8pc2j6qzhyn1nzrzvpghb-cowboy-controller-release`.
The Web predecessor was the independently activated image-send release
`/nix/store/y94qz5696dn6yi9h8kb6q5ffvyaff1lj-cowboy-web-release`.
Neither predecessor was rebuilt or substituted during activation.

Controller PID became **3589153**, start counter **2955308392799**; its executable
matches the immutable ELF above. Web activation preserved that PID/start pair.
Machine PID **2091295**, start counter **2908750440467**, and all **15** running
pre-deployment worker PID/start pairs remained identical across both component
transactions. Worker snapshot SHA-256:
`f63e8ccfdc2e300c16f0c875aa5965f11287af7b749c46e23f85a3bde5c888ff`.
This is the deployment-boundary comparison, not the entire development session.

Machine generation `worker-92b35f0665ec33ba60f6`, its release profile, the host
closure and host receipt stayed unchanged. No Machine/worker, NixOS, native,
Provider, installation or private destination-policy transaction occurred.

## Actual reader floor and public observations

The candidate/predecessor/cold matrix passed **96/96** populated checks before
activation. The independently observed post-activation matrix passed **96/96**
again:

- Controller active and next-transaction rollback: `w024allw` above.
- Controller cold:
  `/nix/store/lynj6vqp7ph7mcn82rwggkwcjzkyn37m-cowboy-controller-release`.
- Machine active and next rollback:
  `/nix/store/gwd83024a8dbi65srphcda55mssbkjh3-cowboy-machine-release`.
- Machine cold:
  `/nix/store/dxwq5k1iab6wsyj7gg24gqndwz9han08-cowboy-machine-bootstrap-release`.

The cold paths came from the actual active host activation script, not inferred
version labels. The completed receipt's `previousRelease` is historical; the
next transaction captures the now-current active profile.

Private create-only final receipt:
`dist/telemetry-reader-conformance/20260913-binding-surface-active.json`,
`accepted=true`, source `43ef8c92`, SHA-256
`166cd824eb994372cbe40d705e7f60feca96ceb6464dde1ac0d012c005d6bbd9`.

Public `/healthz` returned `ok`; Hawk remained connected/online. Root and Service
Worker returned 200/no-store, `/version` and root ETag were
`759e7955e492109b10cb65eb97722341`, SW ETag was
`704fe98894ce09d42f91883bf6fca430`, and SW declared **v1673**. Unauthenticated
choices and ordinary operation-receipt GETs returned 401. Mobile PWA users need
Update to load the new bundle; physical-device interaction is not claimed.

Startup reported `admission_enabled=false managed_namespace=false`; the Machine
binding file remained absent. No production binding confirmation, endpoint
inspection, credential read or OTel test emission was performed. Existing local
recording and explicitly configured legacy export were not replaced by this
staged path.

Build, gate, reader and before/after evidence is retained under
`/tmp/cowboy-binding-surface.fJHZxT/`. Initial default-feature test compilation
and stale integration-test failures were fixed without suppressing gates; their
logs were retained and are not acceptance receipts.

Remaining P2: durable HTTP Machine recovery audit discovery, per-target writer
and background-export policy admission, and production cross-end failure/restart
acceptance. Ordinary durable Service operation GETs do not close the distinct
Machine recovery audit gap; emitted OTel remains `NoRestore`.
