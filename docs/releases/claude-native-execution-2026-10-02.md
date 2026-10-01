# Claude native execution rollout, 2026-10-02

Claude Code Plugin **3.2.0**, using the unmodified native CLI **2.1.287**, is
installed and active on OVH. New Claude sessions can keep their runtime, native
history and subscription authentication on OVH while Hawk or Falcon provides the
execution environment. Existing sessions retain their original placement and
generation.

Source revision `887e9e866ed187e1a7ba491905f87e3c26706a3a` is published on
Cowboy `main`. The [production receipt](claude-native-execution-2026-10-02.json)
records the exact signed digest, original installation operation, retained
authentication and process identities, and hashes of the verification receipts.

## Installation and continuity

The signed release digest is
`sha256:d00b06d70fc102df9844c0aac6def5294f268d63c4e3a0afd8e48421d61c6b3a`. The
exact public Linux artifacts were imported through the Machine-owned cache
command before installation. No installation pointer, live database or
credential was edited directly.

Operation `ovh-claude-3-2-0-native-execution-20261002` lost its initial response
across an observed Machine heartbeat disconnect. The Machine reconnected
automatically. Reconciliation queried that same operation and obtained its
terminal `Applied` receipt without replaying installation; revision:
`installation-83ea0d6722aed7d304584723675274183bf0988b8723ba25521fed77faa7ed1c`.
The Service transaction is completed and no reconciliation fence remains.

Independent inventory reports the exact release active, with authentication
generation **50** retained and both credential replica and materialization
current. The Controller, OVH Machine and both existing worker process IDs and
start times were preserved. No Controller, Web, Machine or NixOS release was
activated. The existing automatic convergence policy and timer remain unchanged.

## Accepted verification

- The complete `just check` gate and final `provider-check` passed, including 12
  focused Claude tests, Rust, Web, PostgreSQL and release validation.
- The
  [actual packaged Claude gate](../experiments/claude-execution-worker-2026-10-02.json)
  passed all 25 checks through the worker/keeper transport: file edits and
  conflict protection, quoted paths and CRLF, images, large results, 35-second
  transport loss without mutation replay, foreground/background cancellation,
  cold resume, real compaction and resume after compaction, ACP new/load, and
  live effort changes. Broken context modules and `--bare` fail before
  inference.
- All six independently packaged Agent releases passed real worker startup,
  teardown and distinct-generation coexistence. Claude was tested against OVH's
  previous installed 3.1.35 generation.
- The exact releases passed 18 Catalog checks across the actual active,
  next-transaction recovery and cold Controller readers. The active profile was
  resolved directly, and the cold reader is in the active NixOS closure.
- All 26 unique published package/runtime URLs matched their bound SHA-256
  values, covering 1,164,465,862 bytes. The live Catalog advertises Claude 3.2.0
  as ready. Service health is normal.

The shared runtime component required separate sibling publications: Codex
3.2.1, Claude DeepSeek 3.1.29, Codex DeepSeek 3.1.29, Gemini 3.1.29 and Grok
3.1.30. This task explicitly installed only standard Claude on OVH. Publishing
those other releases is not an assertion that their Machine slots were upgraded.

## Scope

The
[integration record](../experiments/claude-execution-integration-2026-10-02.md)
explains the native context hooks, non-reserved file tool names and handling of
native image/output/transcript locators. Ordinary commands and file edits need
no model-authored SSH transport or extra routing turn.

This first Claude lane does not enable native subagents, project hooks,
automatic project skills, native plan files or implicit file attachments.
Ancestor guidance is projected literally; imports, nested rules and further
instructions can be read explicitly from the target. PDFs require a target
utility. Background completion is observed through the retained task handle.

Verification used a scripted loopback API and disposable state. It did not send
a real subscription inference request, measure production model-turn latency,
execute native remote sessions on macOS, or test a physical device UI. Existing
native subscription authentication is retained; this release does not replace it
with an API-key integration.
