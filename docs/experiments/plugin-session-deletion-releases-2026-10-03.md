# Immutable Session deletion reader-pair acceptance — October 3

Two independently supplied immutable Machine releases passed a real-process
upgrade/rollback and invalid-storage matrix. They have different source
revisions and native ELF digests, and both declare reader schema 1 with writer
schema 0. The fixture launches their complete `bin/cowboy-machine` wrappers;
it does not substitute a newly compiled test broker for either release.

This is an acceptance-harness change. No production code, writer constructor,
host admission or activation changed, and no service restart was needed.

## Exact artifacts

| Role | Source revision | Immutable release |
| --- | --- | --- |
| Old accepted reader | `ae00cdc884de0cad92f303731ce52541a837efc5` | `/nix/store/vb8kp467y6lmwsa4h6ga5fv0ygf2rkpc-cowboy-machine-release` |
| New accepted reader | `38453c13456bddb4261572ab270354e47b2ffc0e` | `/nix/store/n6b8rxna00v77pyqkyk9658xcwnqnh61-cowboy-machine-release` |

The old native executable SHA-256 is
`afc5ec0806a8690b1ec34932b5c28b50bcca049a72b5432a7e1653dcc24f3041`;
the new native executable SHA-256 is
`aa77267063a5c416ae34415261c0876ab9298c401935ebb7e481b6088a51705e`.
The fixture verifies ELF magic and compares native digests separately from
release and packaged launchers. The
[machine-readable observations](plugin-session-deletion-releases-2026-10-03.json)
retain all three digests, source identity and bounded results.

These are the already published reader releases recorded by the
[reader-floor release ledger](../releases/plugin-session-deletion-floor-2026-10-03.md),
not newly activated components. The test requires canonical direct Nix-store
release paths, clean Machine manifests, schema-1 reader declarations, writer
schema 0, and distinct revisions and native digests. Those checks identify the
test inputs; they do not grant component installation authority.

## Accepted cases

The matrix launches 31 complete Machine processes. Eleven are SIGKILLed and
reaped; twenty exit on the specific expected startup refusal.

- A synthetic schema-1 terminal record survives old → new → old startup and
  SIGKILL. Every reader rejects the surviving worker before Welcome and rejects
  both ordinary and adoption-only EnsureSession with the deletion-specific
  reason. The committed bytes remain unchanged and no worktree is created.
- For each release, an empty or staging-only namespace accepts the existing
  volatile Stop acknowledgement, fences that worker in the current process,
  and permits worker Welcome after a fresh process restart. No committed file
  appears. Invalid staging bytes are neither replayed nor changed.
- Each release refuses malformed JSON, schema 2, foreign Machine, foreign
  Service, duplicate terminal IDs, an unknown field, 4,097 terminal IDs, a
  committed directory, a dangling symlink and a symlink to a valid record.
  Refusal occurs before the broker socket exists. Regular-file bytes, link
  targets and directory contents remain unchanged.

Every Machine has a temporary state directory and its own sockets. Children
receive a cleared environment with temporary HOME/XDG paths. Their Controller
address is an exclusively bound, unserved loopback listener. No real Controller,
Provider package, worker runtime, host activator or production dataset is used.
All startup and IPC waits have deadlines, and every child is reaped on success
or unwinding. The immutable launchers may supply their owned dependency paths
and execution configuration; no environment or agent execution is requested.

The committed records are synthetic reader inputs. They are not production
writer acknowledgements. Private writer crash/ACK behavior remains separately
covered by the [source process fixtures](plugin-session-deletion-process-2026-10-03.md).

## Reproduction and checks

From the repository root, in its pinned Nix development shell:

```sh
COWBOY_TEST_OLD_MACHINE_RELEASE=/nix/store/vb8kp467y6lmwsa4h6ga5fv0ygf2rkpc-cowboy-machine-release \
COWBOY_TEST_NEW_MACHINE_RELEASE=/nix/store/n6b8rxna00v77pyqkyk9658xcwnqnh61-cowboy-machine-release \
cargo test --locked --all-features --test session_deletion_releases -- --ignored --nocapture
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo fmt --check
```

The exact opt-in matrix, Clippy, formatting and diff-whitespace checks passed.
The matrix is intentionally ignored during ordinary tests because independently
supplied release artifacts are required; explicitly invoking it without both
inputs fails. Use the all-feature test command: integration-test selection also
builds this repository's full-feature companion binaries.

## Remaining admission

This is one exact Linux declared-reader pair. It does not start an undeclared
legacy Machine, change the root-owned floor, activate or roll back a production
profile, simulate power loss, or verify a production writer release. Older host
activators, portable updater recovery and general Machine recovery admission
remain open. Writer schema 0 and the production constructor remain unchanged;
the results do not authorize enabling durable production deletion. Continuous
Session incarnation, worktree ownership and native resume remain separate.
