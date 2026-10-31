# Credential probe backoff, usage after re-login, Ultra badge — 2026-10-31

Status: deployed on 2026-10-31; receipts below.

## Incident

On 2026-10-30 the Service (default-account) Claude Code login on OVH expired.
From 14:56 to 15:29 UTC the OVH Machine logged 3,764
`invalid credential refresh excluded from promotion` warnings, about 110 per
minute: each credential-directory event re-ran the native `claude auth status`
probe for the same rejected projection, which stays on disk until it is
replaced. It stopped only when the user signed in again at 15:29.

Usage collection then stayed empty. The authentication failure applied the
one-hour account backoff, Service usage is collected only when a client reads
it, and the re-login did not release the backoff. While a session was live, the
session overlay replaced the collection error ("Sign in to Claude Code again")
with the generic "Account quota is not exposed for this session."

What kept rewriting the credential directory is not established. Re-arming the
watch set was ruled out: an unchanged plan re-armed 20 times emitted no event.
The backoff bounds the probe rate whatever the writer is; a recurrence logs one
warning per retry, which identifies whether the writer persists.

## Changes

- Machine (`96d4eec5`): remember each exact failed candidate (account, Provider,
  expected generation, generation digest and bundle) and retry it after 30 s,
  doubling up to 30 min, in a bounded 64-entry memory. While a candidate is
  backing off, the watcher neither probes nor republishes an unchanged
  inventory.
- Controller (`ce1d5c4d`): a committed default-account credential (login,
  promoted refresh, commit, shared-scope adoption) releases that account's usage
  authentication backoff once and warms collection. The release is in memory
  only and still obeys the Provider window, spacing and account interval;
  Machine reconnects do not trigger it. The session overlay keeps an attempted
  collection's error and uses the empty-state text only for a never-collected
  placeholder.
- Claude Code 3.20.22 (`7ecb11c9`): Anthropic renamed Max 20x to Ultra on
  2026-10-30. CLI 2.1.314 still reports `subscriptionType: "max"`; the collector
  reads the credential's non-secret `rateLimitTier` (`default_claude_max_20x`),
  best effort, and badges the account `ultra`.
- Component release 3.73.0 (`104e602a`) records app-shell 1.1.34 for the
  already-committed spinner changes and Claude Code 3.20.22.

## Verification

| Gate                                                               | Result                                                             |
| ------------------------------------------------------------------ | ------------------------------------------------------------------ |
| `just check-compact` on the change                                 | Passed                                                             |
| `just lint` and server/core unit tests after merging `origin/main` | Passed                                                             |
| `just plugin-check`                                                | Passed                                                             |
| Claude Code 3.20.22 runtime artifacts                              | Byte-identical to 3.20.21 (same digests)                           |
| Live Service usage after install                                   | `planType: "ultra"`; five-hour window absent (lifted by Anthropic) |
| OVH Machine log after activation                                   | No rejected credential refresh                                     |

## Deployment

| Step                | Receipt                                                                                                                                                                                                     |
| ------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Controller          | `89b4cc8b`, committed by the Hawk component activator; health 200                                                                                                                                           |
| OVH Machine host    | `89b4cc8b`, host-only; receipt `/var/lib/columbus/ovh-machine-89b4cc8b0931-20261031T015854Z`; workers retained                                                                                              |
| Hawk Machine        | `89b4cc8b` host release, worker generation `worker-18aa815194c30ab76edd` unchanged; committed                                                                                                               |
| Falcon Machine      | `89b4cc8b` host release, worker generation unchanged; committed                                                                                                                                             |
| macbook-air Machine | `104e602a` built on the Mac, `cowboy-machine-install --refresh` with its existing Service, state directory, capacity, admission and seven workspaces; reconnected online, Plugins converged                 |
| Claude Code 3.20.22 | Signed `cowboy-first-party`, artifact `sha256:ed1d93cd7e07bf7bfd90f122b622d53b821c4c2cb6c5510e6160032fdbd737e1`; installed on OVH (`ovh-claude-code-3-20-22-converge`, completed). No other Machine runs it |

## Remaining risk

The default account's credentials are distributed to every enrolled Machine and
each Provider CLI refreshes on its own; nothing coordinates refreshes across
Machines. In the seven days before this release every promoted refresh for a
Provider came from a single Machine (Claude Code from OVH, Grok from Hawk, none
for Codex), so the race has not been observed.
