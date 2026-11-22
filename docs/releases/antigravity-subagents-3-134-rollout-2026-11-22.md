# Component release 3.134.0 rollout (Antigravity subagents) — 2026-11-22

Status: the seven 3.134.0 Agent Provider releases are published and
`antigravity` 0.2.18 is active on OVH, accepted with real traffic. No Machine
release or worker pin change was needed (`Cargo.lock` and the interface files
are unchanged), and no Controller or Web release: the change is inside the
Plugin's runtime. Hawk, Falcon and macbook-air were not converged (the other six
Providers only take a patch bump for the provider-runtime pin).

## Content

- Antigravity 0.2.18 (`cowboy-antigravity-acp` 1.0.16): the `Subagent` and
  `StopSubagent` facade tools run another native `agy` session inside the
  calling session's turn and report it as an ACP subagent
  ([design](../antigravity-real-accounts-design.md#subagents)). `agy` 1.5.9 and
  1.5.14 declare no subagent capability in `initialize`, so the proxy hosts the
  delegation; native `invoke_subagent` stays denied.
- The other Providers take a patch bump for the provider-runtime pin only.

## Gates

| Gate | Result |
| --- | --- |
| `check-compact` at the merged branch | Passed |
| antigravity-acp tests | 217 passed; the fixes of each review round were mutation-checked |
| Code review (Codex Sol Max, high risk) | Three rounds, the maximum; every finding fixed except per-session backend attribution, which `agy`'s log cannot give (its success lines carry no session id), so evidence is a documented account-scoped contract. The last round's fixes were not re-reviewed |
| `provider-release-coverage` | Passed on the isolated stage and on the live Catalog (7 of 7) |
| `serve --check-plugin-catalog` on the isolated stage | Exit 0, twice |
| Artifact URLs | `antigravity.cowboy-plugin`, `agy-launch.tar.gz`, `agy-proxy.tar.gz` match their content-addressed SHA-256 |

## Publication

| Plugin | Artifact digest |
| --- | --- |
| claude-code 3.20.64 | `sha256:df776eabc18f1598d84ec550f5732bda1bef17c9a762a483f8f3689bd15c39e2` |
| claude-deepseek 3.1.81 | `sha256:521c753ef31cd7fe223fd447e43b631f85d0260ae254bbae6b4384cddec6712a` |
| codex 3.6.23 | `sha256:0b5e9da8f75dfec53712a86d8b5b7ef1d315fbfc80c375471a1faf2edae374bb` |
| codex-deepseek 3.1.81 | `sha256:4ea468422646da90756b6c7e021748acc0c72457034dd109739a815a3f386aae` |
| grok 3.2.22 | `sha256:1ed0d223db22f1b056e91f9a115ba009fd66c7be8dba2b21b7d8f0d36de2ac38` |
| deepseek-harness 1.0.39 | `sha256:f9041de01b5cc3ccf40c39c8f070548fd5dc3165978b2cdfbff071b722e4fd21` |
| antigravity 0.2.18 | `sha256:8fdc72f7abe568b153671335c8887ed2ea42d12d95060c94129054b81f78f7b0` |

## Activation

`cowboy operator converge --machine ovh --plugin antigravity` skipped the
Plugin ("not declared for this Machine"), so it was installed directly:
`ovh-antigravity-0-2-18-subagents-install` completed, Machine receipt
`installation-25b716cc7fb04a0b192d9577d6c46906978335f9cfdb824b12842e6a88948098`
applied, 0.2.18 `active`, rollback generation kept.

## Acceptance (real `agy`, Husky account, preset `gemini-hydrogen-high`)

Session `sess-1795305711880` (Provider 0.2.18, runtime OVH, workspace Hawk):

- Step 1: `Subagent` ("Read README") started a child inside the turn while the
  parent's tool call was open (so `agy` runs a second session concurrently, the
  assumption the unit tests could not prove); the child read the file and the
  report returned to the parent verbatim. Row: `idle`/`end_turn`, 1 tool call,
  model Gemini 4 Hydrogen, prompt and result shown, 11 observed rows.
- Step 2: `Subagent` with `wait_seconds: 0` returned an id at once;
  `StopSubagent` stopped it: row `idle`/`cancelled`, 6 observed rows. The unit's
  `agy` was not ended (the watchdog was disarmed by the answered prompt).
- The parent turn ended normally (`EndTurn`).

A second session, `sess-1795305711881`, ran two children at once (both
`wait_seconds: 0`, then each waited for). Alpha finished (`idle`/`end_turn`, 6
tool calls, `alpha-done`). Beta was stopped from its row
(`sessions.cancel_subagent`, the Web's Stop) while Alpha ran: `idle`/`cancelled`
after 4 tool calls, the parent's wait returned the stopped outcome and the
turn ended normally; the unit's `agy` was not ended.

Not exercised with real traffic: a parent Stop with children running, plan mode
and the fail-closed paths (a child `agy` does not confirm stopped, forced exit,
violations and refusals): unit tests only.
