# Durable installation writer activation — 2026-09-14

Published and activated Cowboy revision:
`4817bc7f6ca095aa8de279ec6c9287261be865e6`.
The preceding [reader floor](plugin-install-receipt-readers-2026-09-14.md)
was accepted before either new writer was enabled. This release enables the
existing protocol-19 Service/Machine path; it introduces no parallel installer,
Plugin package, database migration, production installation or telemetry policy.

## Accepted immutable artifacts

| Lane | Activated release | Transaction |
| --- | --- | --- |
| Controller | `/nix/store/987xsvdchhvh4bhp8qf04680p15zsp7g-cowboy-controller-release` | `1789383517955968199-4817bc7f6ca0` |
| Machine | `/nix/store/pzs2k265fb4klvxd4bx6r5fzsw8harjf-cowboy-machine-release` | `1789383438713508453-4817bc7f6ca0` |

Both transactions are `succeeded`, `committed`, and `published: true`. The
machine-owned component activator first performed the separate Machine
maintenance while Service installation was still paused, then activated the
Controller. Running executable paths match the accepted ELF chains.

The deployment-transaction recovery releases were:

- Controller: `/nix/store/r7hbjdnp82sj6g6bn9s8y9ijhh9iw75p-cowboy-controller-release`
- Machine: `/nix/store/5cvs4mjnaakmc9kl7dg5farsw63zlfr2-cowboy-machine-release`

Those remain the transactions' historical `previousRelease` values, not the
**next** transaction's recovery candidates. After activation, both active and
next-recovery roles point to the new `4817bc7f` releases. Actual cold readers
remain the Hawk closure's `2f7fa237` artifacts:

- Controller: `/nix/store/l4ji30l5pysy8091kwxmz8igsvvpgv95-cowboy-controller-release`
- Machine: `/nix/store/7m0kq3x3hygg5f23l0yg4rcalfh2riw8-cowboy-machine-bootstrap-release`

Hawk's active Columbus revision/closure remain `8e358baba640f361c39f26a49390741cfd3a972e`
and `/nix/store/y9gs82a0pki3r30z1k7r1nv1hx08gwng-nixos-system-hawk-26.05.20260731.5b4f72e`.
No NixOS generation or host policy was changed in this writer activation.

## Gates

`nix develop -c just check-compact` passed on the final code: 1,170 Rust library,
285 standalone Machine and 1,442 Web tests, plus the 17 PostgreSQL checks,
format/lint/dependency/feature/native-shell/site/composition/build gates.
Existing nonfatal Web lint/chunk-size and yanked transitive dependency warnings
were not dependency changes in this release.

The candidate / transaction-recovery / cold artifact matrix passed:

| Gate | Checks |
| --- | ---: |
| Telemetry reader | 96 |
| Telemetry writer admission | 294 |
| Background startup | 78 |
| Service installation reader | 168 |
| Machine installation reader | 72 |
| Connected telemetry faults/delivery | 45 |
| Real isolated Victoria Logs/Metrics/Traces databases | 9 |
| Connected installation writes and cold recovery | 45 |
| Total | 807 |

The new [connected installation gate](../plugin-install-connected-conformance.md)
also passed an independent complete repeat: two accepted runs, each with five
real writer flows and nine independently restored reader pairs, each opened
twice. The real 90-second lost-receipt deadline is unchanged. Same-byte reinstall
produces a new incarnation and exact Installed-target CAS; loss/disconnect does
not synthesize a receipt, replay installation, or release an unknown fence.
Controller crash preserves Installing until each reader independently records
NeedsAttention / Interrupted. Duplicate and changed-input IDs never authorize
another command or target query. No auth/session/export frame is allowed by
this installation fixture's relay.

After activation, freshly captured **actual** active / next-recovery / cold roles
passed another 168 Service and 72 Machine reader checks. These 240 checks are
separate from the transaction matrix above; role labels were not silently
changed in an old receipt.

Initial unsuccessful runs remain recorded, not counted as acceptance:

- The first harness misdecoded SQL's JSON-encoded problem field and relocated a
  CoreSecurity-bound database. Added regression tests preserve the real SQL
  representation and restore each baseline at its original private physical
  path. No authority marker, database identity or production validator was
  rewritten to make relocation succeed.
- One diagnostic recheck, concurrent with builds, timed out at a cold-Controller /
  candidate-Machine startup (two welcomes, one runtime configuration). This was
  not a passing run, and compilation contention is not a proven root cause.
  Both complete final-artifact runs subsequently passed unchanged deadlines;
  no timeout or consistency assertion was relaxed.

## Observed production boundary

Between the 18:56:25 and 18:59:27 +08:00 snapshots:

- All 13 existing worker PID/start-time pairs and all three Victoria process
  PID/start-time pairs were retained. There were no new failed units.
- Controller changed to PID `3820978`, start `3066451876743` monotonic µs;
  Machine changed to PID `3819042`, start `3066372658907`.
- Machine is connected with unchanged `worker-240c2080a8bf9eb8968f` generation.
  This is not a native generation-swap or subsequent-turn acceptance claim.
- Controller startup reports `install_admission_enabled=true`; the running
  writer-capable Machine retains `--plugin-operation-admission`. Original
  Operator, exact release/target, connection and deadline checks still apply.
- Production install-operation count remains zero; Machine
  `install-attempts-v1` remains absent. No production Plugin was installed as a test.
- Applied SQLite migration 23 retains SHA-384
  `c4815f45cee8ace4a1731a75b14cb04efec09f41cf5c0ac5d89c82e6db2b45ce7d884a4853465de6d248325692d679a9`.
  No historical migration bytes or stored checksum were edited.
- `/healthz`, `/version`, SPA/admin/SW bytes and cache headers were checked.
  Web remains `b86032f6`, SPA `55226f7178ca1af323cd60281970697d`, SW `cowboy-v1685`;
  its profile and assets did not change. Public unauthenticated installation
  history still returns 401.
- Existing Victoria configuration/export was not switched to managed mode.
  The private writer/background policies remain unconfigured and their ledger
  absent. Preflight is configuration validation, not Operator or delivery proof.

## Retained proof

Owner-private receipts/logs are under
`/tmp/cowboy-service-install-bridge.fyNuGy`; no credentials, raw database copies
or destination policies belong in Git. SHA-256:

| Evidence | SHA-256 |
| --- | --- |
| `writer-c2-audit.json` | `7885b011808cbe7daa9d4d9961d6f152006605b15f778a75f7d792d10d9551af` |
| `writer-c2-install-connected.json` | `6e00ac019de46022663cd5b6de375234a5c68054ed5614c2980c446433134afd` |
| `writer-c2-install-connected-repeat.json` | `7ff4734eb1dda8485399ac97e3d80c6d278de7db4f7b93b9923614ef81b1a0e7` |
| `post-writer-install.json` | `1620335968ce2d28715b5980095f093c58a424f0523b9f7f5582c7e545335adc` |
| `post-writer-machine.json` | `260d85eae670166c1e2056920b849cbcbc6c0593f1a71c7b88534ce9b472cbf8` |
| `writer-role-audit.json` | `1b4ec67fe1d0e4b4fee5e9528cf018b79ee82a09dbfb32f7949ba558cab96e80` |
| `writer-web-audit.json` | `f0f81139307cf72be6e771e72a3e755ae903c79d259f374c8be51a5925175abf` |

## Not completed by this release

General verified DAG resolution, cross-tab/state-writer coexistence,
independently authorized post-effect restoration and archival, actual Agent/code
capability and native-generation acceptance, production core-security handoff
and managed Victoria cutover remain separate exits. See the
[completion ledger](../plugin-refactor-completion.md). A successful installation
receipt is not proof that all effects are reversible or the refactor is complete.
