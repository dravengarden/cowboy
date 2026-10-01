# OVH Anthropic usage timeout repair

Published source: `f24c04bacf87b5653e6f5bd84b93f45dc5f959c2`.
Production acceptance: three fresh Anthropic account queries returned
`available`, both five-hour and seven-day limits, no error, and `stale=false`.
The permanent Machine remains `ovh`; no enrollment or credential change occurred.

## Root cause

The Controller correctly dispatched Anthropic usage to OVH, but its Machine
Plugin host request exceeded the existing 45-second budget. Before running the
collector, `prepare_usage_sidecars` unconditionally called `inventory()`, even
though Claude declares no usage sidecars. OVH's Agent generations lacked the
optional inventory cache, so this traversed unrelated installed Agents and
reconstructed their inventories through full signed runtime verification.
Archive decompression and comparison ran repeatedly before any Provider request.

A bounded OVH probe of the exact release archive-checking implementation took
7.358 seconds for the two Claude archives alone; entry/finish breakpoints showed
repeated archive checks within target resolution. This is local preparation
work, not evidence of an Anthropic authentication or network failure. The
Controller's pre-fix dispatch/timeout pair at 09:31:21.629624 and 09:32:06.630839
UTC recorded a 45.001-second timeout. Subsequent pre-fix attempts also timed out.

The repair returns an empty prepared-sidecar set before enumerating inventory
when the requested set is empty. Nonempty sidecar requests retain their existing
verification. The target Plugin's signature, digest, runtime checks, and formal
credential materialization remain required. The timeout was not increased and
no cache, live database, or credentials were patched. New structured timing logs
record queue, preparation, command, and total durations without payloads.

## Live acceptance

| Query | Machine preparation | Collector command | Machine total | Operator CLI total |
| --- | ---: | ---: | ---: | ---: |
| 1 | 27.317 s | 1.328 s | 28.646 s | 43.633 s |
| 2 | 27.378 s | 1.328 s | 28.706 s | 29.148 s |
| 3 | 27.079 s | 1.315 s | 28.395 s | 37.111 s |

Each Machine lifecycle queue time was zero. The CLI end-to-end measurement also
includes Controller scheduling and transport; it is not the Plugin-host timer.
Durations are monotonic measurements, not subtraction of cross-host wall clocks.
Fresh observation values changed on all three requests. The remaining target
verification cost is significant: this repair makes the operation fit its
budget, but does not claim instant UI refresh or eliminate all redundant target
verification. There was no Provider error or cached-failure fallback in these
three results.

Reproduce through the existing authenticated operator surface:

```bash
cowboy operator usage-executor
cowboy operator usage --refresh anthropic
cowboy operator inspect --machine ovh
```

Honor the manual refresh cooldown. Do not publish raw responses containing
account identity; retain only status, limit identifiers, timings, and readiness.
Anthropic remains pinned to OVH. OpenAI, xAI, and DeepSeek remain pinned to Hawk;
their final snapshots were available, error-free, and non-stale. Gemini's
pre-existing unsupported account-limit result is outside this repair.

## Deployment boundary and validation

Only OVH's Machine host was replaced. Release:
`/nix/store/lfmg11fp9ygylnmq4x6nrcdm6inxr9w0-cowboy-machine-release`.
Columbus commit `0968e16f` owns the Ubuntu override and finite maintenance helper.
The host uses the release's native `libexec` entrypoint while explicitly
retaining `worker-dd872931e193f388490f` and its old worker executable. This is
not an ACP runtime-generation rollout. Controller, Web, Providers, and other
Machines were not deployed by this repair.

An independently armed four-minute root systemd timer protected the switch.
Eight existing ACP worker PID/start/executable identities survived. The Machine
PID changed from 56151 to 228802. The old Code adapter was replaced separately;
no ACP worker was restarted. Plugin versions, digests, and credential readiness
matched the pre-maintenance inventory. The normal `KillMode=control-group` was
restored after the first successful query, and the owned rollback timer stopped.
This verifies process preservation, not a new interactive conversation test.

The full pinned-shell `just check-compact` passed, and native diff review found
no remaining actionable defect. The new regression first failed without the
fix, then passed in both full and standalone Machine test slices. Integration
initially hit the unrelated inode-reuse assertion in
`the_retained_handle_is_what_makes_inode_comparison_sound`; that failure was
retained and an unchanged complete rerun passed. Columbus passed `just verify`,
nine maintenance safety regressions, native review, native systemd validation,
and a disposable parent/child stop probe before production activation.

The [sanitized receipt](anthropic-usage-timeout-2026-10-01.json) records exact
artifacts, measurements, and retained process identities. Immutable old and new
closures and protected rollback receipts remain on OVH; owned temporary tracing
and transfer artifacts were removed after acceptance.
