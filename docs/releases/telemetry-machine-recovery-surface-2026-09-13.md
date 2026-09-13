# Machine recovery surface acceptance — 2026-09-13

Implementation: `7b73e562f486707425e64f52444bc35e53a9f1cc`. Activated
application source: `a6f20d0096e87fb573f964ad915878ef07aadc23`, retaining the
independent favicon update that reached main during release preparation.

The
[core Machine interruption surface](../telemetry-machine-recovery-surface.md) is
available under Settings → Info for a retained `NeedsAttention` operation.
**Production confirmation remains closed.** Read-only exact previews and
hermetically tested one-use confirmation are not managed writer/export
activation or completed P2. The separate Service confirmation is never chained
automatically.

## Verification and immutable inputs

The complete pinned `just check-compact` passed on the implementation: 1,028
Rust library tests (19 intentionally ignored), 1,397 Web tests, 15 isolated
PostgreSQL tests, formatting, lint, dependency/feature/SDK/Plugin, native-shell,
site and production-build gates. Eighteen type-checked Web surface tests also
passed. Existing OTel spread, dependency and bundle-size warnings did not become
new failure exceptions.

The final favicon integration changed no Rust, Cargo, Nix or worker-generation
input. The site gate and complete Web tests, typecheck, lint and production
build passed again on that revision. The filtered immutable Controller build
passed 810 library tests (15 intentionally ignored) and its binary tests; both
shared public surface fixtures are explicitly included in its source fileset.

Final clean, published release inputs:

- Controller:
  `/nix/store/6jih7xw4jrs8pc2j6qzhyn1nzrzvpghb-cowboy-controller-release`.
- Web: `/nix/store/71fildym8ngp26ydhdhi900wz0hdmc1g-cowboy-web-release`.
- Actual Controller ELF:
  `/nix/store/p9w1j4bdh4720z9n562wjsh5aqhbk4cv-cowboy-0.1.0/bin/cowboy`, SHA-256
  `6320b1c7e48db9bf563feda1fb31fcc482664bdd229a712a87ab93a775aad1ac`.
- Actual Web root:
  `/nix/store/0i0zgrhqvwkz3py4cj1xsxcp53afw3l8-cowboy-web-0.1.0`.

Earlier `7b73e562` artifacts and reader checks are retained diagnostics; they
were not substituted for these final source manifests or live profiles.

## Actual transactions and continuity

This task used the installed machine-owned Controller activator through the
clean isolated Columbus worktree
`/home/draven/worktrees/columbus/cowboy-controller-recovery-20260912`, unchanged
at `de1f6c6504146de3633271fc223d3aaa3c449212`.

Controller transaction `1789266371823109300-a6f20d0096e8` completed with
`succeeded`, `committed`, `published=true` at `2026-09-13T02:26:30.873142624Z`.
Its predecessor was the `5ae8279b` Controller at
`/nix/store/61i3amqivxn0xc0rdxh7gnkmv3i9823r-cowboy-controller-release`. The
detached root unit ended inactive/dead with success and exit status zero; no
incomplete transaction remained.

The same Web revision was already activated by the independent favicon task:
transaction `1789266219143230107-a6f20d0096e8`, succeeded/committed/published at
`2026-09-13T02:23:39.188077133Z`. This task verified that exact result and did
not dispatch a redundant Web transaction or replace it with its earlier bundle.
Its profile stayed unchanged through the Controller release.

Controller PID became 3424568, start counter 2949305764679, and its actual
executable matches the ELF above. Machine PID 2091295/start counter
2908750440467 and all **14** running pre-deployment worker PID/start pairs
remained identical across the Controller transaction. The complete worker
snapshot hash is
`6e5b9e599e60badc3defb74a5cd7ea15b9b9cc9f1de14b77a8e81456aa316deb`. This is a
deployment-boundary comparison, not a claim about every worker that existed
earlier in the development session.

The Machine profile, generation `worker-92b35f0665ec33ba60f6`, host profile and
host receipt stayed unchanged. No Machine/worker, NixOS, native, Provider,
signed Plugin or Catalog transaction was performed.

## Actual reader floor and live observations

The final artifacts passed 96/96 populated reader checks before deployment.
After deployment, the active/next-transaction-rollback/cold matrix passed 96/96
again:

- Controller active and next rollback: the `6jih7xw4` release above.
- Controller cold:
  `/nix/store/lynj6vqp7ph7mcn82rwggkwcjzkyn37m-cowboy-controller-release`.
- Machine active and next rollback:
  `/nix/store/gwd83024a8dbi65srphcda55mssbkjh3-cowboy-machine-release`.
- Machine cold:
  `/nix/store/dxwq5k1iab6wsyj7gg24gqndwz9han08-cowboy-machine-bootstrap-release`.

Profiles and the active host closure's actual absent-profile initialization
paths were independently inspected. The completed Controller receipt's
`previousRelease` is this transaction's history; the next transaction captures
the now-current `6jih7xw4` profile.

Private, create-only final evidence:
`dist/telemetry-reader-conformance/20260913-machine-recovery-surface-active.json`,
source `a6f20d00`, `accepted=true`, SHA-256
`9f2d8ad996db350659e0aeea0b771d07194b7f8358e76a5b1a4b72ec723e9fe2`.

At deployment acceptance, public `/healthz` returned `ok`, Hawk was
connected/online, root and Service Worker returned 200/no-store, and the public
Service Worker identified **v1671**. Root/version ETag was
`02a4b267ef6eccef3e944d86f276494e`, Service Worker ETag
`e89776372c9175628b9f62363c193e0f`. Unauthenticated binding and Machine recovery
receipt reads returned 401. Mobile PWA users need Update to load the new bundle.

Startup reported `admission_enabled=false managed_namespace=false`. The Machine
binding journal remained absent. No private destination/policy/token was read or
changed, no production confirmation was attempted, and no export grant was
issued. Physical-device interaction and production failure/restart acceptance
are not claimed.

Remaining P2 work includes ordinary select/revoke/restore confirmation, durable
HTTP recovery audit discovery, per-target writer/background-policy admission and
production cross-end acceptance. Unknown/schema-one evidence remains quarantined
and emitted OTel remains `NoRestore`.

Build, gate, reader and deployment evidence is retained under
`/tmp/cowboy-machine-recovery-surface.hPLYYZ/`. Earlier fixture-generation, test
and lint failures remain under `/tmp/cowboy-recovery-surface-*`; no failed
attempt was overwritten or counted as acceptance.
