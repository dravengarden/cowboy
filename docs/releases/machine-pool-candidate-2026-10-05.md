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

| Gate | Result | Evidence |
| --- | --- | --- |
| Native Codex worker execution (Codex 0.159.3 executor `8bf204b3…`, packaged Codex plugin 3.3.2 launcher, candidate keeper) | 16 checks accepted | [receipt](../experiments/machine-pool-codex-execution-2026-10-05.json) |
| Diagnostic CLI log conformance | 11 checks accepted | [receipt](../experiments/machine-pool-logs-2026-10-05.json) |

Both used disposable loopback fixtures and no production credential. They do not
cover Claude, coexistence, sessions or Code.

## Not yet accepted, and why

- **Claude native execution and Codex/Claude coexistence.** These need the
  Claude plugin runtime (CLI, `cowboy-configured-cli` launcher) for the repository's
  current Claude plugin. Hawk's installed Claude generations and Catalog stop at
  3.1.x, which predate the execution launcher, and no built runtime or artifact
  root was found locally. The candidate plugin must be built through the
  repository release skill first. The coexistence harness also wants its own
  artifact-root layout and a symlink-free absolute worker path.
- **Execution-session conformance.** The tool in main still sets
  `COWBOY_PUBLIC_ORIGIN=http://127.0.0.1`; the Controller now refuses that
  (`Controller timed out`). A fixture upgrade (HTTPS origin with a local TLS proxy,
  `X-Forwarded-Proto`, device-bound browser proof over the full `/ws?{query}`)
  was reported working on October 4 but is not in main.
- **Connected Code** needs the exact native Zed pair and was not attempted.

Until those pass for this exact worker, the pin must not advance. Use new receipt
paths for every rerun.
