# Controller Provider-auth reconciliation release: 2026-09-14

Hawk's Controller now merges overlapping synchronization of one immutable
Service credential generation on the same authenticated Machine connection.
Implementation and release source: `77756dfa9b785fc1a2c56e78560c650db7937ae2`,
published to remote `main` before activation. No Plugin artifact, protocol,
database format or credential-storage format changed.

## Fix and acceptance boundary

Concurrent Machine reconnects and refresh/projection repair could enqueue
identical `ApplyProviderAuth` commands. A regression test first reproduced two
commands for one pending generation. The
[Core coordinator](../provider-auth-sync-coordination.md) now shares one command,
its original 90-second deadline and actual result. It captures the connection
before the asynchronous enrollment-key lookup, fences replacement connections
and late ACKs, bounds pending owners/observers, and never retries after timeout
or cancellation. Completed results are not cached; later explicit repair and
new generations remain real work.

This is a duplicate-request amplification fix, not a claim that all credential
rollovers were unnecessary. CR-9 still requires a newly accepted generation to
drain matching idle workers and resume their exact native sessions. The Machine
rollover implementation is unchanged. The prior deployment's three Grok PID
changes do not alone prove a defect, and its
[failed all-worker continuity audit](telemetry-policy-preflight-2026-09-14.md)
remains failed and unmodified.

Thirteen new tests cover admission, shared rejection, fresh repair, conflicting
metadata, separate authorities, same-epoch replacement, ACK/cancel races,
resource budgets and cleanup. A virtual-clock test joins at second 89 and
requires the original second-90 timeout without resending. Temporary real
Service/Machine identities exercise vault sealing, signature verification,
replica replay, generation advance and logout wipe. That fixture installs no
Plugin; it does not establish decryption/materialization of an installed
runtime, native-session continuity or production credential convergence.

## Gates and immutable artifacts

`nix develop -c just check-compact` passed: format, Clippy, dependency checks,
independent feature slices, **1,101 Rust library tests** (22 explicitly ignored),
binary/adapter tests, **1,427 Web tests**, **15 isolated PostgreSQL tests**, and
release builds. The thirteen auth-sync tests also passed again from the final
clean commit. The existing Web lint/chunk-size warnings and the dependency
audit's yanked `spin 0.9.8` warning remain warnings; the advisory gate passed.
No dependency upgrade was included in this transport fix.

The exact live Catalog covers all six embedded Agent Plugin releases. The
candidate's configuration-only `serve --check-plugin-hosts` passed as the actual
Service owner using the intended host/authentication/data arguments and public
origin in an isolated network namespace. Its closed telemetry preflight reports
writer/background policies `unconfigured`, with legacy selection explicitly
`not_checked`. It did not open production storage or authorize export.

| Immutable gate | Checks | Result |
| --- | --- | --- |
| Populated Service/Machine readers | 96 | Accepted |
| Independent writer admission/effects | 294 | Accepted |
| Managed Controller startup/recording | 78 | Accepted |
| Authenticated connected managed flows | 45 | Accepted |

All nine Controller/Machine role pairs completed all five flows, including
144 managed-delivery rounds and 324 correlated real RPC receipts/isolated HTTP
requests. The independent audit checked exact source/manifests, actual executable
chain hashes, role assignments, private receipts and required outcomes. These
remain synthetic authenticated identities and an isolated protocol receiver,
not production Operator or real Victoria database acceptance.

Controller release:
`/nix/store/3wbj6ky4kqp5b88fiv7p3h2pxb28lifn-cowboy-controller-release`.
Actual ELF SHA-256:
`df7e22c249e00e85860ff1f0790e41b25b9958f7884215946ec3eabbfe73b4d5`.

The accepted planned matrix assigns this artifact to the new active and
next-transaction rollback roles. This transaction's automatic recovery target
was the then-active
`/nix/store/xa07qj5xx6q72lrvnnmdnmj7yyxdw9kj-cowboy-controller-release`.
Its retained 96/294/78/45-check receipts were independently re-audited and its
matrix matched a fresh host capture before activation. Historical
`previousRelease` is not substituted for the next transaction's target.

Cold Controller and active/rollback/cold Machine roles remain those recorded in
the [previous release](telemetry-policy-preflight-2026-09-14.md). Columbus remains
`1da44eb88883b52a7bff266da0c07ede35193659`, host closure
`/nix/store/ki6hs9zi8w14q42rzj2xyksg6hxckxg8-nixos-system-hawk-26.05.20260731.5b4f72e`.

## Production activation and bounded observations

Only the machine-owned Controller activator was dispatched. Transaction
`1789346881168195446-77756dfa9b78` committed successfully at
`2026-09-14T08:48:24+08:00`, with `published: true`. Controller PID/start changed
from `1663934` / `3026147307505` to `1814627` / `3029815133421`; its live
executable resolves to the exact accepted ELF. No NixOS, Machine, worker
generation, Web, Catalog or host-policy activation was performed.

Captures at `08:47:44`, `08:49:11` and `08:50:34` (+08:00) agree on all **13
worker PID/start pairs**, Machine PID/start `110025` / `2971136905262`, Machine
and Web profiles/receipts, host closure/receipt and Web root. The strict after
and settled audits both passed. No new failed system/user unit appeared.
`/healthz`, online Machine presence and active worker generation
`worker-92b35f0665ec33ba60f6` passed. Root/SW cache headers and ETags agree.
`/version` remains SPA hash `ae371d1395b4ad7a2ceda868237026b5`, not the
Controller Git revision.

The bounded journal observation for that 170-second window counted zero auth
promotions, stale refreshes, replica sync failures, refresh rejections or repair
failures. It records counters only, not credential contents or raw logs. No
production credential generation advanced in that observed window: this cannot
accept real rollover/resume, a subsequent user turn, complete credential
convergence or the number of coalesced production requests. The prior failed
continuity evidence is not replaced by this successful new window.

## Retained evidence and remaining work

Private evidence: `/tmp/cowboy-auth-sync.lYXmus/`, including `quality-2.log`,
`auth-sync-committed.log`, `build.log`, four gate logs, preflight, independent
receipt/after/settled audits, three host captures and bounded observations.
The intentional red reproduction, first compile/type correction, feature-slice
fixture correction and initial unpinned capture's missing `jq` failure are
retained separately. No failing runtime oracle or timeout was weakened. The
corrected capture used the pinned shell and a new directory.

| Receipt under `dist/` | SHA-256 |
| --- | --- |
| `telemetry-connected-conformance/20260914-auth-sync-1.json` | `c73f8b618cd1743fefd00b913ef21d865ff2ba1f1ecfc5f6d0106de2f9b548f3` |
| `telemetry-writer-conformance/20260914-auth-sync-1.json` | `00bb77f425bba091784373562529b69dec7f95dd594524374f71cbd52b5a9ef1` |
| `telemetry-reader-conformance/20260914-auth-sync-1.json` | `4554e0a693bb94c4248d0db287ada35748253e5a7228d35004be08312d7b732b` |
| `telemetry-background-startup-conformance/20260914-auth-sync-1.json` | `8e12e71f84274bbc66ecc273a3496872360bf7c473e98f35e20eeebd9f91c643` |

Matrix: `dd550ba18634ad1a2cfd18130997202dadbd310dd0922184a8f6b0d7df3909ff`.
Preflight: `a4dc87351f50d90a05231cc784b19eec46766ac58b720341478c87330d3568fd`.
Worker observation: `8d3fbc6392127145ac78f9924854e986d17221e631c0a8cc4b5dc077a8f9c645`.
Auth observation: `3e851a451c86c129bc5111165ee7e9f0fbc346b6174c04c4d4fbbde5096ff516`.

The production managed Victoria cutover still needs complete intended
Service/Machine policies, fresh actual Operator confirmation, real
ingestion/query and failure/restart acceptance. Existing legacy Victoria export
is not disabled by this release; managed admission remains closed. First managed
intent permanently fences the legacy path, even if refused or aborted. Do not
open writers alone or erase evidence to regain it. P2 and later generic Plugin
DAG/P3/P4 migration remain unfinished.
