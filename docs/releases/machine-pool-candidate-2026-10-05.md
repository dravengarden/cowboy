# Machine worker-pool candidate — October 5 (partial acceptance, not activated)

Purpose: advance the retained `cowboy-workers` pin from `b97c2724`
(`worker-748825b42b4302fe26ca`) so host-only Machine releases can carry main again.
Main contains the independent wire change `22de6bbf` (host resources and idle
hibernation), which `retained-worker-interface-compatible` correctly refuses, so no
resident-only release can be built until the pin advances. This follows the
`0b8e116b` precedent: full candidate, independent pool acceptance, then pin.

Candidate source `d47a32d72f98451bb1c2b6911ac686b9a5b0d930` (published main);
`.#cowboy-machine-release` is
`/nix/store/caiv4bjk766yl6la34fqbkgvci5h9d1r-cowboy-machine-release`, worker
generation **`worker-5e00906640eeac46d44c`**, declaring deletion reader 1/0 and
incarnation reader 1/0. Nothing was activated and the pin was not advanced.

## Consequence to accept before activation

A new desired generation makes the Machine drain every live worker whose
generation differs: idle workers are replaced at once and busy ones after their
current turn, restored through native resume. On Hawk that is every active Session,
including concurrent tasks. This is the actual native-generation replacement the
ledger still lists as unaccepted, and it needs the explicit go the user gave for
this work ("a") together with a fresh review of the live worker list.

## Accepted on the candidate

All with the exact candidate worker `cowboy-acp-worker` (executable digest
`sha256:b2350db1…`, previous pool `1838792b…`) or candidate Machine/Controller
bytes, disposable loopback fixtures and no production credential.

| Gate | Result | Evidence |
| --- | --- | --- |
| Native Codex worker execution (Codex 0.159.3 executor `8bf204b3…`, packaged Codex 3.3.2 launcher, candidate keeper) | 16 checks accepted | [receipt](../experiments/machine-pool-codex-execution-2026-10-05.json) |
| Diagnostic CLI log conformance | 11 checks accepted | [receipt](../experiments/machine-pool-logs-2026-10-05.json) |
| Public sessions, enrolled Machines and signed ACP fixture, with device-bound browser proof (`browser_device`) | 9 checks accepted | [receipt](../experiments/machine-pool-session-2026-10-05.json) |
| Codex generation coexistence: Codex 3.3.2 (`a833d5b6…`) with previous 3.3.1 (`d5b985f3…`) | accepted: initialize and session/new, stop and descendant drain, distinct-generation coexistence, no sidecar | [receipt](../experiments/machine-pool-codex-coexistence-2026-10-05.json) |
| Claude generation coexistence: the Claude Code releases **installed on Hawk**, 3.1.35 (`051263cb…`) with previous 3.1.34 (`6b00907a…`) | accepted, same checks | [receipt](../experiments/machine-pool-claude-coexistence-2026-10-05.json) |

Input notes, from the reruns: the session tool needs `"browser_device": true`
(without it the Controller refuses its HTTP origin and the harness reports a
Controller timeout) and the native `.cowboy-machine-wrapped` ELF rather than the
release wrapper, which already passes `--desired-generation`; coexistence needs
the harness layout (`<release>.release.json`, sibling `.cowboy-plugin`, and
`artifacts/artifacts/<digest>/<file>` as regular files, passed as the artifact
root) and a symlink-free absolute worker path; every rerun needs new receipt
paths.

## Not yet accepted, and why

- **Claude native execution, and coexistence for the repository's current Claude
  plugin.** The coexistence above covers what Hawk runs (3.1.x). The repository's
  Claude plugin is 3.4.7 with the remote-execution launcher
  (`cowboy-configured-cli`), which no Hawk generation or Catalog entry contains.
  Native Claude execution and its coexistence need that runtime built through the
  release skill, including the CLI download.
- **Connected Code** needs the exact native Zed pair and was not attempted.

Until the Claude native execution and connected Code gates pass for this exact
worker, the pin must not advance. The activation consequence above is unchanged.
