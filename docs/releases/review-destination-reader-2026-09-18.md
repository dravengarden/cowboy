# Review-owned destination reader — 2026-09-18

**Published on main; activated on Web only.** The
[Review reader](../plugin-review-owned-destinations.md) connects the existing
original-source navigation, opaque target handoff and independent native-text
owner to an explicit read-only target view. Production navigation acquisition
remains default closed. This is not a native rollout or refactor completion.

## Source and immutable release

Implementation: `cb4d81580c257085cf8bdf4558824ef357651d0b`, based on fresh
`ff74cfca05b10a71514d22ca8f9e79da3495c876`. The following documentation commit
records acceptance without changing the executable release.

- Web release: `/nix/store/k5id6dfd5vpvymi15y6x4rwhz4zr2mj2-cowboy-web-release`.
- Assets: `/nix/store/6mknq7l32y0lsjahhxmcjnbp8vyd9agr-cowboy-web-0.1.0`.
- SPA version: `ad171718d697327435290a0198bb5c9a`.
- Service worker: `cowboy-v1719`.
- Source-boundary check:
  `/nix/store/sf1qav345bggfd5fkn8ifkg5wvjmhjs2-cowboy-source-boundary`.

No dependency, SDK/Plugin version, native/core wire contract, database migration,
durable namespace or host policy changed. Native immutable inputs remain the
previously accepted exact artifacts, not binaries relabelled with this Web SHA.

## Implemented boundaries

Prepare serializes behind source reads and binds the authentic displayed
capture and UTF-16 point; acquisition is a separate explicit one-use action.
Replacing the source/point or leaving the view fences late Execute. Five local
target choices appear per page without effects or capacity allocation.

Reading a target reserves its original slot, opens it once, and displays only
verified complete native text with both actual UTF-16 endpoints checked. No
path read, partial render, legacy fallback, automatic retry or native reload is
available. Lost handoff/Open can only inspect the original operation, with
separate explicit Open/read when permitted. A second consumer cannot take over
the existing child's cleanup. Group release preserves an opened target; closing
the target does not release its group.

Navigation hides old source intelligence. After group retirement, explicit
Check must re-observe the original source and complete content before restoring
annotations; it never replays Open. Settings → About adds a passive paged
original-group Query/confirmed-Release projection, including lost-response
recovery after the originating view has left. Release is not rollback or proof
of physical native-buffer closure.

## Acceptance on the clean implementation

Pinned-shell `just check-compact` passed: format, Clippy, strict Web type/lint,
dependency/component/Plugin/feature checks, PostgreSQL and release builds.
Existing telemetry lint, yanked `spin` and large-bundle warnings remain visible.

| Suite | Passed |
| --- | ---: |
| Main Rust library | 1,478 |
| Codex app-server bridge | 3 |
| Standalone Machine | 367 |
| Core Code adapter | 26 |
| Private Zed adapter | 102 |
| Web | 1,794 |
| Isolated PostgreSQL | 18 |

Main/Machine/private adapter retain 34/four/two explicitly ignored tests. Twelve
new unit cases include source cancellation, complete target integrity, UTF-16
refusal, duplicate consumers, lost/pending operations and late cleanup.

All **81** Firefox `151.0.1` cases passed with fresh profiles/private loopback:

| Browser suite | Cases | Fixture SHA-256 |
| --- | ---: | --- |
| Buffer/content/navigation owner | 24 | `13f3891c7e77f0224f3df354e9ec4ba2b841f3bacd8270949cd9a9b6ce1b256f` |
| Product context | 6 | `fcd8c031bfb4e8324d8cde8d3c3ab15bd127c50d397213a39fd89d623773ba92` |
| Ordinary cleanup | 7 | `bb93243818d657ab64416c0d949e6d32e711f8f90d888f7d3fa8498e9efe5f2f` |
| Synchronization | 8 | `b8d0bc6c23c926bf47072fb2f450f81f889183b3b201c1cfcf6904d94e87d35e` |
| Review source | 6 | `d09d3de646ab97188e1765dd18f58be008d14ece488beb1398df297182acba5b` |
| Review diff | 6 | `242aad9f9a491f7c2911a0863ff000fede3e2d74a0407ecd46b6a710baba5b95` |
| Review document refresh | 5 | `c2c3f71a07e033419e5efa3de758eae61995408c0edd4a57d5abbc9217fec4b3` |
| Review destination and navigation recovery | 10 | `2ad9f3bd726cca3eef859a75c73d0236ea74d74ef3b498d4c7c14b7e1e0be51c` |
| Settings recovery | 9 | `5d4323f1bd75b91f162b937be19dae2b237a649cc44643160b82d3d6a773e87f` |

The new suite mounts actual hooks, React development StrictMode, MUI and
CodeMirror with real browser WebCrypto; HTTP alone is synthetic. It checks
explicit clicks, rendered text/range, source reconciliation, independent group
release, closing during handoff, mismatch, access loss, passive confirmation /
lost Release and local pagination. It is not physical-device acceptance.

Connected native v5 passed **all 18 checks**, `accepted=true`, `cleanup=true`,
`stage=complete`, `failure=null`, in 130.65 seconds. Three genuine replies were
discarded with unchanged transport timeouts; cleanup reaped the fixture native,
Machine and Controller processes. The supplied pair is unchanged from
[the browser destination candidate](browser-destination-candidate-2026-09-18.md):

- Controller `/nix/store/mvjc17wlqa0w6mlb9miwk5d60548aly8-cowboy-controller-release`;
  Machine `/nix/store/jxmg5ln0qglblmp5k5m380b6ck3ya9xq-cowboy-machine-release`,
  both source `f2d41397c04f64ca1cb1565841ca6d3c9bc85891`.
- Zed adapter `1.13.2`, SHA-256
  `b5d4afd9bfe6aebf208beeea1f889ebbaa5755040ee6230f7d66849e8c1222b3`;
  server `1.0.0`, SHA-256
  `da41ec6baee1cbf714b809fcd912dd200be150c634cbe4a6ec6939e323da8131`.
- Receipt SHA-256:
  `31dcf234aad99bdd8c1b744d1bd32c2446b03570b998fd23944646d804e44939`.

Fixture installation, forced teardown and a synthetic test LSP do not establish
production installation, language implementation freshness or recovery.

## Web activation and bounded observations

Owned transaction `1789707434830105129-cb4d81580c25` completed with
`outcome=succeeded`, `phase=committed`, `published=true`, `maintenance=false`.
Predecessor: `/nix/store/i5mwgzd376rgik3y9y6s6j2ll8nmwzb9-cowboy-web-release`.
No recovery override, Controller/Machine activation or Plugin installation was
issued. The activator performed its routine old Web profile-generation pruning;
the predecessor remains recorded for recovery, and no user content was deleted.

The **12:57:06–12:57:32 +08:00** observations retain all **18 worker** and the
Controller, Machine and three Victoria PID/start-time identities. Profiles,
host receipt/system, workspace identity and failed-unit sets are unchanged
except the intended Web profile. Machine remains online on
`worker-9c07ddc149d208d345e5`. Both the private navigation environment setting
and command-line override are absent, leaving production admission default
closed; neither was changed.

Correction: the original Code process-name sample contained zero rows before
and after. Equal empty files do **not** prove native Code process continuity.
That overbroad field in the first task audit is withdrawn and superseded by
`activation-audit-corrected.json`; native Code continuity is not accepted here.
The other compared process records are present with nonzero PIDs.
Corrected audit SHA-256:
`f38025b89af17b63b8472c6f350e71357697f644fb7aa4f77b16d2f15d63d616`.

Local and public `/healthz` and `/version` pass. Index, admin, service worker and
both entry JavaScript files match the immutable release byte-for-byte on both
origins, with no-store/immutable cache headers checked. Installed clients must
load v1719; mobile follows its existing explicit Update flow. This is not
authenticated production Review or supported-device reload acceptance.

Private task evidence: `/tmp/cowboy-review-destination-PBgWls`, including
`full-final.log`, nine `*-accepted.log` browser receipts, `connected-final.json`,
immutable build logs, before/after observations and the corrected activation
audit. No production credential was copied or used.

Native pre-acquisition budgets, exact registered native generation / intended
consumer device acceptance, independent post-effect recovery and the general
graph/state-lease exits remain in the [completion ledger](../plugin-refactor-completion.md).
