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

## Production acceptance

Hawk and OVH are activated. The [release receipt](native-calls-images-2026-10-18.json)
records exact immutable paths, source revisions, transaction IDs, signed Plugin
digests and all 13 completed/applied installation receipts with re-read inventory.

| Role | Accepted source | Result |
| --- | --- | --- |
| Hawk cold recovery | Columbus `b57888c7`, Cowboy `aca5ad97` | Host transaction succeeded; no changed service units |
| Controller | Cowboy `337c3674` | Committed; health and version endpoints healthy; compatible explicit recovery retained |
| Hawk Machine | Cowboy `337c3674` writer host | Committed; generation `worker-7d0bb83659a51cd6f7e6`; writer enabled |
| OVH Machine | Cowboy `337c3674` reader host | Finite maintenance accepted; same generation; all 44 original native worker processes retained at acceptance |

The exact two-writer production gate passed 45 groups, and connected Code passed
38 checks, including the real lost-reply deadlines. Its Controller and ordinary
Machine native executables are byte-identical to the deployed release; the
documentation commits change release provenance, not those bytes. Both actual
cold inputs and the explicit Hawk recovery closures understand the image
purpose/profile. OVH keeps the current compatible override as
`accepted-recovery.conf` in its protected maintenance receipt; `previous.conf`
is only the historical pre-admission predecessor, not a post-image recovery
target. The independent rollback timer was stopped after acceptance.

All ten signed versions are ready in the live Catalog. Seven installed Plugins
on Hawk and six on OVH were upgraded and each exact version/digest was confirmed
against its terminal Machine receipt and live inventory. Service-managed Plugin
membership requires the delegated host Operator CLI. The initial MCP refusal
occurred before approval or installer admission; absence of that domain operation
was checked, and the host CLI reused the same domain operation ID. Agent reads
and supported control continue to prefer MCP. Existing session/Tools policies
still govern Calls admission; a Codex parent uses its own native image tool
rather than delegating back to the same Provider through Calls.

These production observations establish installation, runtime generation and
recovery readiness. No live image model was prompted and no billed token/image
usage was measured during this rollout. The native Provider still checks the
actual account, supported model and image quota when an agent requests an image.
