# Telemetry binding reader/transport rollout — 2026-09-12

Cowboy `3faf554b5c37541109caad21bce2ba2f1f0de4a5` is published to remote main
and accepted on Hawk as both Controller and Machine. Machine maintenance was
explicitly confirmed after the Controller release. Both telemetry binding writer
gates remain closed. This is not production binding activation or P2 completion;
the [protocol contract](../telemetry-binding-wire.md) describes the implemented
behavior and remaining requirements.

## Immutable releases

- Controller:
  `/nix/store/w55cm4nvnav1f58bgr8xgyvwxrqaiv7d-cowboy-controller-release`.
- Machine: `/nix/store/9zvsf5f507d7jd504a8ad82hq7yhlcpc-cowboy-machine-release`.
- Source-boundary check:
  `/nix/store/s8ndn0hm0w6hcr8cdzqkj4ha4a3nrar5-cowboy-source-boundary`.

The clean source passed `nix develop -c just check-compact`: 959 Rust library
tests passed, 16 intentionally ignored; 1,369 Web tests and 14 isolated
PostgreSQL tests passed. Both immutable outputs and the filtered-source check
built successfully. The real cross-component lost-ACK fixture waited for the
45-second RPC timeout and queried the original step without resending. These are
hermetic write tests, not production mutation acceptance.

## Owned activation receipts

The clean isolated Columbus worktree at
`42b3845233ebba35874fdc712c19a5400eb05876` supplied the existing component
activator via its `candidate` entrypoint. Both repositories were freshly
fetched. No NixOS switch, direct profile manipulation or manual worker stop was
performed.

Controller transaction `1789204098510489331-3faf554b5c37` committed successfully
at `2026-09-12T09:08:31.453015365Z`, with `published=true` and
`maintenance=false`. Its predecessor is
`/nix/store/sfkgvwlrp5anyx6gvzgyg9f6ap6gd1nh-cowboy-controller-release`.

Machine transaction `1789210701770854596-3faf554b5c37` committed successfully at
`2026-09-12T10:58:31.10486016Z`, with `published=true` and `maintenance=true`.
Its predecessor is
`/nix/store/kb64pqgsibh9q3x47pv81a5r04pix9bj-cowboy-machine-release`. Receipts
are owned by `/var/lib/hawk-component-deployments/cowboy-controller/` and
`/var/lib/hawk-component-deployments/cowboy-machine/`; neither lane has an
incomplete transaction after acceptance.

## Live observations

- The Machine authenticated to the Controller with protocol 15. Its reported
  active generation is exactly `worker-92b35f0665ec33ba60f6`, matching the
  immutable manifest. Its actual executable resolves into the new Machine
  package, not the predecessor.
- Controller PID/start time did not change during Machine maintenance. All 14
  pre-maintenance worker PID/start records were retained in the post-activation
  snapshot; 15 worker units were active at that observation. This does **not**
  prove every existing worker has adopted the new generation. Workers with a
  current turn or pending permissions retain their old generation until a safe
  boundary; no session was stopped merely to make the rollout appear complete.
- `/healthz` returned `ok`; Machine presence was online. The workspace revision
  and workspace-ID hash were unchanged. The public root and service worker
  returned HTTP 200 with `cache-control: no-store` and unchanged ETags. The Web
  version remained `cafcbe0bd1a020d9a1baac0966a328e3`.
- Controller startup reported `admission_enabled=false managed_namespace=false`.
  Machine `plugin-operations/telemetry-bindings-v1.json` remained absent as both
  a file and a symlink. No binding namespace or export grant was created.

The worker fingerprint changed because the integrated Codex 3.1.18 manifests
participate in the existing core worker fingerprint. Machine maintenance did not
install that Provider, change a Provider authentication generation, or manually
rebind a session. Provider installation remains a separate operation. No private
telemetry policy, credentials or native history were used as release evidence or
modified by this task.

Live readers now exist on both Sites. The accepted rollback predecessors and
cold recovery inputs still need schema-two binding reader acceptance before any
writer can open. Managed per-attempt export leases, explicit write admission,
independently authorized interruption resolution and live cross-end recovery
acceptance remain unfinished. Already emitted OTel data cannot be reverted.

Detailed bounded observations are retained in
`/tmp/cowboy-binding-wire.ora9sL/acceptance.md` and
`/tmp/cowboy-binding-machine.Xsjhpi/`.
