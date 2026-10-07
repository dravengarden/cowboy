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
(`wrong_observation`, after 17 checks). The same stage fails identically on `main`
`6bc5b1df`, so it predates this change; its cause is unknown. It remains open.

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
