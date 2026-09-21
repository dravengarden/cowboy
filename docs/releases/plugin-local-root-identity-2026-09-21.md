# Controller-owned identity for locally executed reads — 2026-09-21

Published and activated the Controller-only
[Controller-owned identity for locally executed reads](../plugin-local-root-identity.md).
It states the other half of the rule the
[Machine-owned fence](plugin-machine-workspace-identity-2026-09-20.md) began:
the party that touches the filesystem owns the root's continuous identity.

A colocated Machine's advertised roots, and a colocated or standalone `local`
Session's worktree, are read by the Controller itself and never reach a
Machine, so the Machine-owned fence never applied to them. **The primary
deployment is exactly that shape**: its resident Machine runs in local mode,
so all 32 roots it exports and every session worktree it prepares were read
directly by the Controller with nothing observing the object behind them.

The Controller now observes the object when it takes the observation and
re-observes at the two gates that choose local execution. A replaced root
retires a Workspace observation and makes a Session route unequal to the
current one, so cached bytes, ETags and page/diff continuations stop
answering. Resolution never refuses: an unobservable root is recorded as such
and refused only where it would be read.

## Exact release

- Final clean source: `24b8891f2e83cde3e2b9adb98fef6f4838f60eb3`, pushed to
  remote main before activation.
- Controller: `/nix/store/3drs6cdhjdh97lyvya2a9qb0s0214ypj-cowboy-controller-release`.
- Previous Controller: `/nix/store/iyq3cvywv6y401440sbxbyaxaqd2438v-cowboy-controller-release`,
  source `bb2c955cc441…`.
- Activation `1789954165875350154-24b8891f2e83` committed at
  `2026-09-21T01:29:51.109591582Z`: published, succeeded, non-maintenance,
  no recovery.

No Plugin/SDK version, Machine wire protocol, native binary, SQL baseline,
durable format, host policy, production role, credential, Machine generation
or Web release changed.

## Why creation time, and why not a retained handle

The Machine pins each advertised root with an open directory handle so the
kernel cannot reuse its inode. The Controller cannot: its soft open-file limit
is 1024 while one colocated Machine may export up to 1,024 roots, and those
descriptors would compete with its listeners and connections.

The Controller therefore compares device, inode **and creation time**, and
requires the creation time. This is not a theoretical concern: on the deployed
filesystem a directory deleted and recreated at the same path reuses its inode
**immediately**, which a source test demonstrates, so device and inode alone
would have passed the fence. This detects reuse rather than preventing it, and
depends on the filesystem recording a birth time; a root that cannot report
one is refused for local execution rather than read unfenced.

## Gates

- Complete pinned-shell `just check-compact` passed on the final source:
  1,633 main Rust tests (34 ignored), 384 standalone Machine tests (4
  ignored), 26 core adapter tests, 126 private adapter tests (2 ignored),
  1,874 Web tests and all 18 separately isolated PostgreSQL tests. Formatting,
  lint, types, dependency, feature/build gates and all 86 structural-link
  vectors passed.
- Four source regressions, each verified to fail against the previous
  behaviour by restoring exactly that behaviour and re-running: a replaced
  colocated root refuses local execution and retires its scope; a replaced
  `local`/colocated Session root ends the route and refuses execution; and two
  cases where an unobservable root still resolves a route but never executes
  locally. Nine further tests cover minting, removal, non-directories,
  relative paths, neighbour independence and content changes inside a root.
- Connected v12 accepts all **32** checks against the published pair,
  unchanged, with three protocol-22 connections, 21 held real replies and
  seven discarded through their normal 40-second product timeouts. That chain
  covers the remote branch of every function this change touches.
- Candidate, active, next-transaction recovery and actual cold Controllers
  each passed two isolated reads of all 88 signed Catalog releases and the
  actual Service host-policy preflight, before and after activation; the
  settled snapshot also re-checks the replaced predecessor.

## The coverage this does not have

The connected gate does **not** cover the colocated branch, and the gap is
structural. Its Machine reaches the Controller over TCP, and `colocated` is
derived from the Machine's self-declared `connection_mode`; a fixture could
only become colocated by declaring local mode over TCP, which is the trust gap
below rather than a property worth building acceptance on. Until that is
fixed, **the colocated branch has source-test evidence only**. This is weaker
than the preceding slice, which had an actual-process negative, and it is
recorded rather than papered over.

## Trust gap found while building this, not fixed

`colocated` — the switch that makes the Controller read its own filesystem —
is taken verbatim from the Machine's self-declared `hello.connection_mode`,
with no transport, peer or enrollment check anywhere. Enrollment deliberately
writes `connection_mode = 'outbound_wss'`; the connect path then overwrites it
from the hello. Any enrolled Machine can declare local mode over TCP and have
the Controller read Controller-host paths that the same Machine chose, through
the authenticated Code read API. The Machine is enrolled and challenge-signed,
so this is a trust-boundary crossing rather than an unauthenticated attack
surface. It is unexploited and unfixed; closing it needs its own slice, because
binding local execution to a real transport or to enrollment changes how the
deployed Machine connects.

## Bounded production observation

Between `2026-09-21T01:29:12.388Z` and `2026-09-21T01:30:06.192Z`, all **19**
ACP workers retain their exact PID and kernel start ticks. The resident Machine
remains PID `3706189`, kernel start `349734872`, protocol 21. Both resident
Code native processes, all installed Plugin generations, all three Victoria
units and the Machine and Web receipts are unchanged. Only `cowboy.service`
restarted, from PID `2998534` to `250339`; `hawk`, `falcon` and `macbook-air`
all reconnected within the window.

Local and public `/healthz`, `/version`, `/` and `/sw.js` return 200, and the
SPA and service-worker bytes are byte-identical across the window. No refusal
from this change appears in the Controller journal after activation.

Before activation, all 32 roots the deployment advertises were observed: 30
resolve to directories with a creation time and are unaffected. The two that
refuse — `deepseek-harness-cloudflare` and `lasso` — are already-absent
directories whose reads fail today, so only their refusal reason changes. No
authenticated production read was driven through the deployed Controller; that
remains a gap in this record as it was in the previous one.

## Evidence and remaining exits

Private evidence root: `/tmp/cowboy-root-identity.SxGe3WWW`, final-artifact
evidence under `published-local/`.

| Evidence | SHA-256 |
| --- | --- |
| `local-check-final.log` | `5ba07d6b319cd128007e08a5eb9a36fc9f56b691d906fd4835c840a6ab91b1c7` |
| `local/connected-receipt.json` | `4f1435e47ffd7f159b16378a782c8a5960f88c0dc9436d26a0b4c9f441c8c82f` |
| `published-local/connected-receipt.json` | `10e8ede3ec05bdeecdbe803c2fd920a6f9476fcdc37b947af6b7419280d32afa` |
| `published-local/check-final.log` | `c5ed0f09bad3d002d4540e0d2dbb97630f9abe8ee90966fbf942787ac22eb9bc` |
| `published-local/floor-deploy/floor.json` | `aea509fc4f0cfa9e8fab643c8458b05fbb74b7c085c210d7c83ab1e7d261fde2` |
| `published-local/floor-after/floor.json` | `8e3122e6c63b02abf313e0279f984147590c37688b61bdf57da91f0e86c7dd12` |
| `published-local/observed-before.json` | `f550ac5f6d1638ecbd7f5f37ff82ccd366d2e77a067b98242bccdbb2f58b44f9` |
| `published-local/observed-after.json` | `c8c10c08258c21a91d92f3ad07559b3b35cdf0411abfe54a551efa23c58443b4` |
| `published-local/activation.log` | `e8de913633f21a439d71188b956c5cc4e7ec422b5f0a78b784ec7672a4fd9c60` |

This closes the local half of one root-object fence. It does not close the
trust gap above, connected coverage of the colocated branch, Machine-owned
Session or security-domain identity, state reader/writer leases, general graph
contracts, independently authorized post-effect restoration,
native-generation replacement or supported-device acceptance. A refusal is an
ended observation: never a rollback, an undo, or proof that a read already
under way has stopped. Those remain in the
[completion ledger](../plugin-refactor-completion.md#code-work-still-required).
