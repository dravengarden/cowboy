# Hawk Zed remote-edit admission installation — 2026-09-19

The separately authorized **Hawk Zed 1.19.0 → 1.20.0 upgrade is complete**.
Service evidence is `completed`, Machine evidence is `applied`, and current
inventory is active on the exact signed digest and new installation revision.
All **19 original Cowboy process identities**, including the Controller,
resident Machine and **17 ACP workers**, remain in the bounded observation
window. No component restart, Machine maintenance or other Plugin upgrade was
performed.

This installs the already signed
[remote-edit admission release](native-remote-edit-budgets-2026-09-19.md), built
from `b5be5b752ca9612a311a043a5d4690717a16c4f9`. It does not rebuild, re-sign or
republish those bytes. Private adapter changes from `1.19.0` to `1.20.0` and
private server from `1.5.0` to `1.6.0`; upstream remains
`aaf5f57dd36c41cf2ed49b13bcb091d52d5aef45` with unchanged third-party pins.
The platform remains Linux x86_64, component release `3.11.0`, payload schema 2,
outer release schema 1 and Machine protocol 21. An Agent authentication-contract
fingerprint does not apply to this Code Plugin; no login or auth state changed.

## Exact release and one installation operation

The repository-owned `converge-machine.ts hawk --plugin zed` dry run selected
exactly one step, from the current Catalog. Before dispatch, the target must
be the authorized `1.20.0` release, not a newly resolved moving latest version.
The published envelope is byte-identical to the retained accepted release;
the immutable SDK verifier independently validates it against the configured
`cowboy-first-party` public key. Catalog identity, ready state and platform
must agree with the signed envelope.

Preflight also requires zero Zed session leases, enabled installation admission,
no reconciliation requirement and the original installation revision. Existing
host delegation is used without rotation. The intent is saved before dispatch;
one normal delegated `cowboy operator upgrade` submits its exact version/digest
and the convergence script's deterministic operation ID.

| Identity | Exact value |
| --- | --- |
| Plugin / adapter | `1.20.0` |
| Private server | `1.6.0` |
| Composite SHA-256 | `b000481cf7548af7598bad0498141daca1d1e422efa2ac1dd90d8cb00830b65f` |
| Contract fingerprint SHA-256 | `b317168255b68ce092c035cb90648986984c3de06b24da26e3a7bcc3c92ab6a9` |
| Signed envelope SHA-256 | `7876a44235e7f2b466a743b5bc048c89f26d839f444aca9a3295816144290675` |
| Operation ID | `hawk-zed-1-20-0-converge` |
| Installed revision | `installation-5c8a4bb8b57984df0016565512f99668696decf85c1b6570e4da847df63d9f5f` |
| Retained previous digest | `04bec5bfe03fbc6aeabe1701102dbbe24d3840007d7bbdbf1087bee83664279a` |

The single call returns HTTP **204**. The Service row was created at
`2026-09-19T05:16:25.215Z` and completed at `2026-09-19T05:16:34.320Z`.
Read-only acceptance at `2026-09-19T05:17:01.557Z` verifies the original operation,
`problem=null`, Machine `applied` revision and matching active installation.
There is no replay, alternate operation ID, database edit or installation-pointer
write. Retaining the preceding generation is not an executed rollback or
independently authorized restoration.

## Bounded production continuity

The baseline at `2026-09-19T05:15:39.011Z`, installed observation at
`2026-09-19T05:17:14.718Z` and settled observation at
`2026-09-19T05:17:59.375Z` retain:

- All 19 original PID/start/executable identities, including all 17 original
  ACP workers. Additional processes may start normally; none of the originals
  is replaced in this window.
- Controller `bc28e8b0`, PID `2621243`, at
  `/nix/store/a64y42ivj4xbjf5bbavbganxmazzi868-cowboy-controller-release`.
- Machine `7269199e`, PID `2129810`, generation
  `worker-3a3c8b33f96774b1981e`, at
  `/nix/store/rg1266170l053dkzb292b2hrlxq8xhyh-cowboy-machine-release`.
- Web `bc28e8b0` at
  `/nix/store/40cbaba6a5b84agdjkz4lpidycxs824a-cowboy-web-release`; local/public
  SPA version remains `df246b07357c80149e12e316cb6cfe7e`.
- The three component receipts, host closure, three Victoria process identities
  and every unrelated Plugin installation/auth-generation projection.

Local/public health is HTTP 200 and Machine deployment health reports connected,
online, on the same generation. A second read-only convergence plan has empty
`steps`, `skipped` and `remaining`; it submits no operation.

This window begins after the independent deployments recorded in the original
publication receipt. It does not repair or relabel that earlier failed
continuity observation. In particular, unchanged processes here do not imply
that every process from the earlier publication baseline survived.

## Evidence and limits

The publication's complete source gate, 57 native tests, exact static pair,
temporary signed lifecycle, two 19-check connected v6 runs, 24 browser checks
and 89-release reader floor remain retained publication evidence, not new runs
against the production Controller. This installation adds fresh signature,
intent, durable receipt, inventory, convergence and health/continuity checks.
No production text was edited and no physical-device test was performed.

The core Operator lock repair remains undeployed by this task: its separate
Controller gate still lacks five exact Agent publications, as recorded in the
release receipt. Private navigation policy, existing native-owner adoption,
global resource limits, independently authorized post-effect recovery and the
broader [completion exits](../plugin-refactor-completion.md) are unchanged.
Whole-refactor completion is not claimed.

The later [Agent publication and Hawk rollout](agent-upgrades-2026-09-19.md)
closes those five publication gaps and deploys the Controller repair without
changing this installed Zed generation. It has its own acceptance window and
does not revise this earlier installation evidence.

Private evidence directory: `/tmp/cowboy-zed-remote-edit-install.8n1HZz`.

| Evidence file | SHA-256 |
| --- | --- |
| `installation-intent.json` | `4997c083b099ee34daebacdcbed4b4eee028ea688d08014a3c0f138d03f33e17` |
| `installation-response.json` | `99816853c0cdcf9fd26763db42527fe3000ca889234b6ff7e98842671d1146ca` |
| `installation-acceptance.json` | `57f9596b8130fd5fa6187a2e9d13846aed8cd78b6aecd821232236013a28bddc` |
| `audit-before.json` | `47b18dea3e1fec9c5d169161ddc42db29940481294ff932dd4db5978df3eae37` |
| `audit-installed.json` | `432a4d7cdb6a0a5ad565cdfb033422e4062e7c7cba65fc71753293beee692d9f` |
| `audit-settled.json` | `7d8ecffac9a22b95f62b7b9615d5717a9f95fafad16c7ca6051b9285e0eb3d12` |
| `plan-after.json` | `9ff23b3f48df954228cd8366fba525648e7ea9ce133df2b577ca51f5802cf019` |
