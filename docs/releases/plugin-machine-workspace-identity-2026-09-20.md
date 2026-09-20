# Machine-owned Workspace root identity — 2026-09-20

Published and activated the Controller-only
[Machine-owned Workspace root identity](../plugin-machine-workspace-identity.md).
An advertised root now has an owner. The Machine mints one opaque incarnation
per root object, pins that object with a retained directory handle, and
re-checks it on every workspace-scoped Code request before reading. The
Controller carries the value and nothing else: it cannot construct, derive,
renew or default one. The Machine's closed `WorkspaceRootIdentityChanged`
refusal retires exactly that observation, so Controller caches, ETags and
page/diff continuations stop answering for a replaced root too.

This closes a real consistency hole. Before it, deleting and recreating an
advertised root — or swapping it for another worktree or mount at the same
absolute path — left id, canonical path and connection unchanged, so the
Controller kept one logical read sequence running across two unrelated
filesystem objects.

Machine protocol is now 22. Against a protocol-21 Machine the Controller
advertises and carries nothing and behaves exactly as before, so this
Controller-only activation changes no production behaviour until the separate
Machine maintenance boundary. That boundary is not scheduled by this task.

## Exact release

- Final clean source: `634fabbae907d3e7c8ce3a1f88cfb61453a387c8`, pushed to
  remote main before activation.
- Controller: `/nix/store/l4wc57brz5xgxyd43ivg1156226pw9x2-cowboy-controller-release`.
- Executable: `/nix/store/hx3m05xipxhmgv4rzkfllzz1w3cy18ag-cowboy-0.1.0/bin/cowboy`;
  SHA-256 `959e8a8c2da0b9ed911e1c2564c901d6eb68781085c54b6160c35ba9df034c5e`.
- Previous Controller: `/nix/store/sq5mk7qvsjwa8bsazf414069769sj7a5-cowboy-controller-release`,
  source `8ce2d403f143bcd4e29b913c033c092d314f5df9`.
- Activation `1789890327337352129-634fabbae907` committed at
  `2026-09-20T07:45:50.450101529Z`: published, succeeded, non-maintenance,
  no recovery.

An earlier candidate built from `1c20061b` passed the complete quality gate,
two full connected chains, the recorded negative and 24 browser cases. Remote
main advanced twice during that work, so the final artifact was rebuilt from
the exact pushed revision and passed the complete quality gate and the
connected chain again. The intermediate candidate was never activated.

No Plugin/SDK version, native binary, SQL baseline, durable format, host
policy, production role, credential, Machine generation or Web release
changed. Private navigation admission and the single-user permission mutation
API remain closed.

## Gates

- Complete pinned-shell `just check-compact` passed on the final source:
  1,610 main Rust tests (34 explicitly ignored), 384 standalone Machine tests
  (4 ignored), 26 core adapter tests, 126 private adapter tests (2 ignored),
  1,870 Web tests and all 18 separately isolated PostgreSQL tests. Formatting,
  lint, types, dependency, feature/build gates and all 86 structural-link
  vectors passed.
- Source negatives, each verified to fail against the previous implementation
  by restoring exactly that behaviour and re-running:
  `a_new_machine_owned_incarnation_ends_the_old_observation` (the old code
  preserved the identical scope across a changed incarnation) and
  `a_machine_root_identity_refusal_retires_exactly_that_observation` (the old
  code kept the slot, and therefore its caches, alive until the next
  inventory). Twelve further source tests cover minting, the retained-handle
  pin, removed and replaced roots, dropped and re-added configuration,
  unusable carries, budget bounds, the protocol floor for a carried value and
  the protocol-21 Machine's unchanged behaviour.
- The retained-handle pin has its own test proving the mechanism is necessary:
  the same delete/recreate sequence hands an unpinned directory back its old
  inode number, so device and inode alone would have passed the fence.
- Connected v12 accepts all 32 checks against the final published pair in
  291.64 seconds, with three protocol-22 connections, 21 held real replies,
  seven discarded through their normal 40-second product timeouts, one
  deliberate cut and passing fixture cleanup. The intermediate candidate
  passed the same 32 checks twice (291.51 s and 291.62 s).
- Connected negative: the immutable production Controller
  `/nix/store/azgn6wvmb1b2qq2y9nal85j8l8sajsai-cowboy-controller-release`
  with the same supplied new Machine negotiates protocol 21, carries no
  identity, and serves the replaced advertised root with HTTP **200** where
  the corrected Controller returns `410/no-store` without an ETag. It fails at
  `machine_owned_root_identity`; cleanup passed. The relay deliberately admits
  the older negotiation so this reaches the product assertion instead of being
  rejected at the handshake; receipt acceptance still requires protocol 22 on
  every observed connection.
- Isolated Firefox `151.0.1` passed all 24 buffer-owner cases, fixture SHA-256
  `4e7a11c5ac7d212fa57370ad912bca000d784143febfa4d683eece9bd7b2da27`. This
  fresh-profile fixture is not physical-device acceptance.
- Candidate, active, next-transaction recovery and actual cold Controllers
  each passed two isolated reads of all **88** signed Catalog releases and the
  actual Service-owned host-policy preflight, before and after activation. The
  settled snapshot also re-checks the replaced predecessor. Managed telemetry
  writer and standing background policy remain unconfigured.

The supplied Machine is
`/nix/store/bmv2yvzpqgfj1ipiq2w3ajx4m5cdspi2-cowboy-machine-release`, source
`634fabbae907d3e7c8ce3a1f88cfb61453a387c8`, protocol 22. It is a **gate
input only**; the resident Machine is unchanged. The native adapter is
`/nix/store/ng88l6jhmpnk07rxjwmz2s00l6llnwk0-cowboy-zed-adapter-x86_64-unknown-linux-musl-1.20.0/bin/cowboy-zed-adapter`
(SHA-256 `19ab0d291ccd51d9049d17eb88659effd3ee60634ec2365e20e77d8aff8b14ec`),
and the server is
`/nix/store/i9yxq38r7pzhdkmp4bq8sypihaqlnxpq-cowboy-zed-server-x86_64-unknown-linux-musl-1.6.0/bin/cowboy-zed-server`
(SHA-256 `f0d700155fb4b6fc1b25ec96d33d53ecd31e2278cab93f783def78003826930c`).

## A defect this work introduced and fixed before release

The first implementation carried root identities inside the Machine's watched
workspace *configuration* snapshot. Every subscriber of that snapshot restarts
the Code adapter, so re-observing a replaced root recycled the adapter and the
next read failed with `502`. The connected gate caught it; no production
component ever ran that code. Identities now live only in the owning registry
and are refreshed exactly when the Machine advertises them, and a source test
pins the invariant: replacing a root's object must not look like a
configuration change, because it changes no trusted path. The Machine also now
logs which advertised root ended an observation.

## Bounded production observation

Between `2026-09-20T07:44:43.839Z` and `2026-09-20T07:46:03.801Z`, all **15**
original ACP workers retain their exact PID and kernel start ticks. The
resident Machine remains PID `3706189`, kernel start `349734872`, release
`/nix/store/gh05dprk2zk8wqp6kjsiyn1ss30sdwb5-cowboy-machine-release`, source
`9ec6d7207f7a520085455c1e1f2a8036fcc278d8`, generation
`worker-f07cb63df88e764816f3`, protocol 21. Both resident Code native
processes (`44463`, `44490`), all nine installed Plugin generations and all
three Victoria units keep their identities. Only `cowboy.service` restarted,
from PID `2387427` to `2517463`; the Machine reconnected within the same
window. No retained worker was force-replaced.

A later settled observation at `2026-09-20T07:48:47.737Z` — outside that
window — shows the Controller, Machine, both Code processes and every Plugin
generation still unchanged, with 14 of those 15 workers retained: one exited
and one unrelated worker started afterwards, both with start ticks later than
the activation. That is ordinary session lifecycle, not deployment continuity,
and it is recorded here rather than folded into the window above.

Local and public `/healthz`, `/version`, `/` and `/sw.js` return 200. SPA and
service-worker bytes are byte-identical across the window and the Web version
remains `e026fca4a2aea9156ef8f80419041eb2`. The Web release and its receipt
(`fa3ec67e2d90…`) belong to a separate, independently activated deployment and
are not attributed to this Controller release.

## A pre-existing gap recorded, not introduced or fixed here

The Controller lane's `accepted-recovery` pin
(`/nix/store/hcdzb1vpvb7mm61pawqywgv36m4a12iw-cowboy-controller-release`,
2026-09-07) cannot decode the current Catalog at all: it rejects the
`telemetry_backend` Plugin kind and does not accept the current preflight
invocation. Both facts predate this change by weeks and are unrelated to it.
The actual cold role used by recent releases reads the full Catalog and passes
its host preflight. Refreshing that pin is a separate machine-owned decision.

## Evidence and remaining exits

Private evidence root: `/tmp/cowboy-root-identity.SxGe3WWW`. Final-artifact
evidence is under `published/`.

| Evidence | SHA-256 |
| --- | --- |
| `negative.log` | `2cc4a480b90cdd5c1b2612c98562f79284857ae803f04f1a402c95527b840d11` |
| `negative-receipt.json` | `f3fe723e778f35399a7262f5d7ecb66ba2a7287b1332f70cb429dcc23ef43122` |
| `connected-receipt.json` | `93f77b94e0d41fc1010ed41e67a7a7f99b91065e71a8d586bb1f6cac6b82fc5f` |
| `connected-second-receipt.json` | `cbd180cdb9086b56f2cbcb2d66b779952d6d07db7a67f67ad60f7af46cd794af` |
| `browser.log` | `d07339320b7701b36c87b47fff09426f9f474efc23780b699efe4001f1a09995` |
| `check-final.log` | `50ab958f1307151557546f7ce3f07bdb7e8db8f2f79639216d1f0c8e39dee3f9` |
| `floor-deploy/floor.json` | `a815129a343bdb3e20521ee49213fd02b3fb790973f663a0947f1bc593221056` |
| `published/connected-receipt.json` | `d479885888dcbe9b36ec7a69c25ac987ef5746644b27e5cc2fc563b9b60a2cae` |
| `published/check-final.log` | `d88f140b6405eb45c26503f8162180c77641f6da95c1a7e434f0cb669f127f6e` |
| `published/floor-after/floor.json` | `31fabd0a6f832936978b1c96ec5ffff9491e05dca914207b0c770b128db852c0` |
| `published/observed-before.json` | `9fcd8a8e05fd54002e01cc43c69e2143707d5de8a18297abc0e620519f371146` |
| `published/observed-after.json` | `bfd92d9212dc8b259676c7caf5e7b4754d6a1ef748b16491c27170a04ef52900` |
| `published/observed-settled.json` | `2aafd691849f7c2127b42df3ccf2dc8886836a078a7137bb00a35d3962822ba5` |
| `published/activation.log` | `74d00dd8e43a2ed86b09f5619840197b070f5daca3b9735f4fad2236447d2012` |

This closes one root-object fence for advertised Workspace reads. It does not
close Session routes executing in session worktrees, Controller-local
(colocated) execution, intermediate Machine configuration changes, continuous
Machine-owned Session or security-domain identity, state reader/writer leases,
general graph contracts, independently authorized post-effect restoration,
native-generation replacement or supported-device acceptance. A refusal is an
ended observation: it is never a rollback, an undo, or proof that a read the
Code adapter already started has stopped. Those remain in the
[completion ledger](../plugin-refactor-completion.md#code-work-still-required).
