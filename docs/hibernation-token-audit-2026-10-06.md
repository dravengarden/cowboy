# Hibernation and wake: token-cost audit (2026-10-06)

The user's condition for the idle-session hibernation feature (`22de6bbf`,
protocol 26) is that it must not waste tokens, and that nothing may be installed on
the assumption that it does not. This page records what was actually checked. It is
an audit of the code on main, not an activation or a policy decision: hibernation is
not deployed on Hawk and no idle-timeout policy exists or has been approved.

## Checked

- **Hibernate sends no model request.** `Broker::hibernate_session` refuses a busy
  worker (running turn, pending permission, background work, queued prompt), then
  stops the worker, forgets the Machine's declaration and reports `Exited` with the
  resumable native ID. It issues no prompt, compaction or title request.
- **Nothing relaunches it automatically.** The Controller keeps its declaration,
  but on every reconnect it replays it with `adopt_only`, which never launches a
  worker. Only an explicit open or prompt revives it. There is no automatic idle
  policy, so there is no hibernate/wake cycle to spend anything.
- **Wake restores state without a model request**, measured against the real pinned
  Codex 0.159.3 and the packaged ACP adapter with a scripted loopback model API:
  - native `thread/resume` after a cold restart: zero requests;
  - packaged ACP `session/load` plus configuration replay on a new process: zero
    requests;
  - the following real turn executes once and preserves history, binding and
    effects (existing checks).
  Both assertions are now permanent checks in `tools/execution_worker_conformance.py`
  (`cold_native_resume_makes_zero_model_requests`,
  `packaged_acp_wake_makes_zero_model_requests`); the accepted receipt is
  [here](experiments/hibernation-wake-codex-zero-model-2026-10-06.json) (24 checks,
  31 scripted requests, 0 real).
- **Claude** already asserts that resume readiness makes zero model requests in
  `tools/execution_claude_worker_conformance.py`; it was not re-run for this audit.

## Not established

- **The next real prompt after a wake is not free of extra cost.** It replays the
  restored history like any prompt, and a provider-side prompt cache that expired
  while the session slept is a cache miss. That is the ordinary cost of idling, but
  hibernation cannot be said to cost zero tokens end to end, and no real-provider
  measurement was made (no credential was used).
- **DeepSeek cache protection (found and fixed).** The provider gateway keeps a
  replay snapshot per session and, while it exists, replays it with real model
  requests (`request_purpose: cache_keepalive`, `adaptive-replay-v1`). Delete,
  reset, provider roll and configuration change revoke it; `hibernate_session` did
  not, so a hibernated DeepSeek session could keep paying for cache keep-alives.
  Hibernation now revokes it (`session_hibernated`), covered by a test that fails
  without the call and shows other providers are untouched. The gateway is in the
  signed Plugin runtime, outside this repository, so its actual replay behavior for
  a revoked or stopped session was not observed; this is a conservative fix from
  the call graph, not a measured token saving. It is active on Hawk since the
  [2026-10-06 release](releases/hibernation-cache-revoke-2026-10-06.md); no real
  DeepSeek session has been hibernated to observe it.
- **Gemini, Grok and DeepSeek** resume paths were not measured; the zero-request
  wake assertions cover Codex (and Claude's existing check) only.
- **Claude** and the fixtures use scripted APIs; this is not supported-device or
  production evidence.
- **Policy.** Idle timeouts, thresholds and automatic hibernation are unapproved.
  The capacity prompt and manual-hibernate UI are another task's work.

Activation of the feature still requires the worker-pool maintenance described in
the [pool candidate](releases/machine-pool-candidate-2026-10-05.md), which drains
every live worker, and the user's explicit go.
