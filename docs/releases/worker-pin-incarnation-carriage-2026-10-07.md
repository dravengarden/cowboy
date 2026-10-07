# Worker pin advance and incarnation carriage — Hawk, 2026-10-07

## Source

- Carriage `abf08c88` merged with fresh `origin/main` as `4039df61`; full gate green
  (fmt, both clippy sets, 2636 tests passed, 0 failed).
- Pin commit `f469cf04`: `cowboy-workers` moved from `5b3547a6` to `abf08c88`
  (worker generation `worker-95d6504dc9c75698606c`). Both commits were pushed to `main`.

## Acceptance before activation

Against the candidate at `abf08c88`: Codex execution, session (9) and logs (11)
conformance passed; Codex coexistence passed; Claude coexistence was not applicable
(Hawk has no Claude plugin generations directory). Native session-deletion production
conformance passed, 45 groups, between two writer builds and two reader-only builds
that carry the new pin.

**Connected Code failed** at `owned_read_failed_reply_original_login_revocation`
(`wrong_observation`, after 17 checks) before activation. The same stage failed
identically on `main` `6bc5b1df`, so it predated this change.

**Cause, found afterwards (test harness, not product):** the connected relay in
`code_connected/proxy.rs` refuses every Machine event it does not list. Since
`22de6bbf` the Machine sends a periodic `HostResources` event; the relay treated it
as a wrong observation and dropped the connection about 13 s into the lost-reply
step, so the Controller answered 401 after 13 s instead of the 40 s command timeout
the step asserts (measured: 13.06 s before, 40.006 s after). Allowing
`HostResources` in the relay fixes it. With that change the full connected Code
conformance is accepted against the deployed Controller `f469cf04` and the
pin-candidate Machine release `11rn7nyz…` (receipt
[`code-connected-host-resources-2026-10-07.json`](../experiments/code-connected-host-resources-2026-10-07.json)).
That run used the pin candidate Machine, not the activated writer-host release, and
the exact native Zed pair already named in the input.

## Activation

| Lane | Release | Receipt |
|---|---|---|
| Machine | writer host release `swm6lxsq…`, `--maintenance` | `succeeded`, `committed`, revision `f469cf04`, generation `worker-95d6504d…` |
| Controller | `p6c4yn5k…` | `succeeded`, `committed`, revision `f469cf04`, `maintenance: false` |

Machine PID `3696691` → `470023`; Controller PID `168944` → `548200`. Startup logs
show `writer_enabled=true`, 7 incarnations, 7 deletion entries, 0 pending cleanups.
`/healthz`, `/version`, the SPA, the service worker and Machine deployment-health
returned 200 after each lane.

## Continuity

All 7 `cowboy-acp-worker` processes kept their PIDs (diff of before/after lists is
empty) and the keeper count stayed 22. **No drain happened**: the activation did not
replace live workers, contrary to the earlier runbook text, which is now corrected.
They will be recycled as their sessions revive; new launches use the new generation.

## Not observed

- The carried lineage is process-local on the Controller and not exposed, so its
  stamping and consumption were not seen in production, only in tests.
- A reset rotating a lineage and a deletion ending one on a live Session.
- Gemini and Grok resume paths; Claude native execution for the current Claude plugin.
