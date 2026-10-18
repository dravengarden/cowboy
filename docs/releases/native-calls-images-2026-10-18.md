# Native image Calls and extensible capability routing — 2026-10-18

## Source and ownership

Native image generation/editing is a Calls capability, using the existing four
MCP tools and the ordinary parent-scoped lifecycle. The caller chooses when to
discover and invoke it. The reusable extension contract is in
[managed native capabilities](../managed-native-capabilities.md); new capability
work must update that contract and its focused tests as behavior changes.

Implementation and reviewed persistence fixes are in
`2e295524433901e469079f405da6fe0625a3388a`. Maintenance pin commit
`aca5ad9751ac33edc9d257008e12f202f32db809` selects that worker source,
generation `worker-7d0bb83659a51cd6f7e6`. Later documentation and Web integration
commits retain the same native runtime and signed Plugin bytes. Codex owns its
native tool, credentials and conversation; Cowboy owns scoped routing, durable
Calls and independently verified delivery to the execution Machine.

## Prepared Plugin closure

Component release 3.56.0 contains Codex 3.5.0, Claude Code 3.19.44,
Claude DeepSeek 3.1.36, Codex DeepSeek 3.1.35, Gemini 3.1.35, Grok 3.1.37,
GitHub 0.2.4, Matrix 1.0.1, Review 1.0.4 and Zed 1.20.8. The ten candidates
were signed by `cowboy-first-party` and verified independently with the trusted
publisher public key. Native Codex CLI 0.160.1 and ACP 2.2.2 stay pinned.
Candidate packages are preparation evidence until the live Catalog and each
Machine's installation receipt advertise the exact version and digest.

## Validation before admission

The full repository gate passed, followed by targeted persistence/Clippy and
fresh-main Web checks. The final native image fixture passed 13 checks,
including cold continuation without a model request, exact input reads,
verified PNG delivery, idempotent delivery and API-key refusal. It uses a
scripted loopback service, not a live image account, so it proves neither billed
usage nor real image quota availability.

Exact immutable artifacts additionally passed Codex worker (36 checks), Claude
managed execution (7), Claude worker (93), keeper (12) and complete session
lifecycle (17). All six Agent Provider old/new generation coexistence checks
passed without submitting a model prompt. The 30-case Catalog reader matrix
proved safe future-package skips and visibility to the candidate reader; it
does not accept old strict readers for persisted image purpose/profile records.
The final Zed packaged install/drain and immutable native-sync gates passed.

Worker-pin admission also requires the runbook's native production writer test
on two distinct committed writer builds carrying this same worker generation,
the actual reader-only/cold artifacts, and connected Code acceptance. Preserve
the resulting exact receipts with the release record.

## Rollout and recovery

The user accepted Hawk then OVH maintenance on 2026-10-18. Hawk keeps the
schema-one deletion writer through a writer-host release; its cold Machine
remains read-only and retains the new accepted workers. Update the actual cold
Controller and Machine inputs before admitting image-capable Providers.
Both runtime and execution Machines need protocol 33. Use the installed owning
activators and inspect completed receipts, not dispatch acknowledgements.

OVH receives only verified runtime closure paths built on Hawk. Use its finite
worker-maintenance helper, independent rollback deadline and exact retained or
native-generation-handoff checks. Never restart its whole worker control group
or build there as a shortcut. Busy native tasks retain their existing process;
report observed replacement or retention rather than assuming all workers drain.

Once image records exist, recovery Controllers and workers must understand
`image_v1`, `image_generate` and `image_edit`. Catalog skip compatibility alone
does not make an older Controller a valid recovery target. Keep accepted reader
closures and history; never remove records or relax reader guards to downgrade.

Production activation and live inference results are recorded after their
receipts have been inspected. Preparation checks above are not a claim that
the feature is already available on a registered Machine.
