# Call capabilities

Status: contract for [background tasks](background-tasks.md) Phase 4, first
slice. Not implemented.

## The gap

Calls already is a common lifecycle: admission, placement, receipts, cancellation
and verified artifacts ([managed native capabilities](managed-native-capabilities.md)).
What stands between it and "a model exposes what it can do, a person chooses what
a session may use, and the model decides when to use it" is narrower than a new
platform:

1. **Policy is per agent, not per capability.** A session can allow Codex but
   cannot allow Codex's images and refuse its reviews.
2. **`same_agent` applies to every capability.** It exists so a second opinion
   comes from another family, yet it also stops a Codex session from generating an
   image through Codex, which no other installed Provider offers.
3. **Discovery reports capabilities per Provider, not their state.** A model
   cannot see which capabilities this session may use without trying them.
4. **Capability identity is implicit.** A record holds a `purpose`, not the
   versioned capability that was admitted.

Capabilities still come from the fixed list in the Provider SDK
(`ManagedRuntimeProfile::capabilities`), and `Purpose` is a closed wire enum read
by the Machine. Opening those is not part of this slice. See
[Later slices](#later-slices).

## Native coverage and the residual gap

Neither Codex nor Claude Code routes a task to another Provider's account on
another Machine, so per-capability policy across Providers stays Cowboy's. When a
native capability registry offers the same scoped routing, this layer goes away.
Claude is shared: the policy, discovery and admission are Controller-side and do
not depend on the calling runtime.

## Identity

A capability has an id and a version: `review@1`, `design_review@1`, `analysis@1`,
`image.generate@1`, `image.edit@1`. The Controller holds one table:

```text
id, version, default_enabled, second_opinion
```

| id | default | second opinion |
|---|---|---|
| `review` | on | yes |
| `design_review` | on | yes |
| `analysis` | on | yes |
| `image.generate` | on | no |
| `image.edit` | on | no |

Every existing capability stays on, so no session changes behavior at upgrade. A
capability added later defaults off. An id this Controller does not know is
listed as unknown by discovery and is refused at admission.

`second_opinion` carries the family rule: only those capabilities refuse the
caller's own family, and auto selection for them keeps preferring another family.
For the others the caller's family is an ordinary target when policy allows it.

## Policy

```text
capabilities:<agent>           {"<id>": bool}   Owner defaults, per calling agent
session_capabilities:<session> {"<id>": bool}   Session override
```

- These are separate settings keys rather than fields of `AgentTools` and
  `SessionTools`, whose strict parsing would make a rollback lose or reject the
  stored policy. An older Controller ignores them.
- The effective value is session override, then Owner default, then the table.
  An override holds only what differs and is removed when empty.
- `calls.enabled` and the allowed targets still apply first. A capability cannot
  grant what they refuse.
- `GET`/`PUT /api/sessions/{id}/tools` and `/api/agent-tools/{agent}` carry a
  sibling `capabilities` object. `tools.session_set` accepts it through the same
  writer.
- **An agent cannot widen itself.** `override_widens` also treats enabling a
  capability as widening, so an agent-submitted change may only turn capabilities
  off. A person's change is unrestricted.
- Continuations and duplicated sessions copy the override like the rest of
  `SessionTools`, under the same widening check.

## Admission

After the existing checks, the request's capability is resolved from its
`purpose`, then:

- disabled: `capability_disabled`, `admission: not_submitted`. No approval prompt
  in this slice: the existing prompt describes an agent target, and a person
  changes this in Tools.
- not offered by the selected target's signed profile: the existing
  `unsupported_capability`.
- own family: `same_agent` only when `second_opinion`.

Auto selection considers only targets that offer the capability, so
`provider:"auto"` with `image.generate` reaches Codex instead of refusing because
another family is preferred. An explicit target is honored or refused, never
replaced.

A self-call for an image is not recursion: the image profile has no Cowboy tools.
The target's concurrency and total budgets apply as they do today.

The accepted record gains the label `capability: <id>@<version>`. `labels` already
persist and replay, and the record's `provider_generation_digest` already pins the
manifest that declared the capability, so no schema or wire change is needed.
Older records have no label and remain readable.

## Discovery

`calls.capabilities()` gains a small list:

```json
{"capabilities": [
  {"id": "image.generate", "version": 1, "enabled": true, "providers": ["codex"]},
  {"id": "review", "version": 1, "enabled": false, "reason": "capability_disabled",
   "providers": ["codex"]}
]}
```

`enabled` is the session's policy. `providers` are targets whose installed signed
profile offers it and that policy allows. It reads installed state only: no model
request and no quota refresh. The existing per-Provider fields stay.

The model decides: nothing here routes a task or adds to resident prompts.

## Product

The session's Tools section and Settings → Agent tools list the capabilities the
installed Providers offer, each with a switch and the Providers offering it. Copy
names what it does ("Generate images"), not the id. The section keeps its folded
layout, and a session override shows a way back to the default, like other fields.

## Rollout and rollback

- Controller, then Web. No Machine release and no migration.
- Reader and writer ship together: an older Web ignores the sibling object, and
  its `PUT` omits it, which leaves the stored override untouched.
- **Rollback widens.** An older Controller ignores these keys, so a capability a
  person turned off is available again until the Controller returns. The rollback
  note must say so.

## Acceptance

Automated: defaults keep behavior; session over Owner over table; `calls.enabled`
and target refusals outrank capability state; agent-submitted enable refused,
disable accepted; person's enable accepted; continuation copy; `capability_disabled`
before any record; `same_agent` for second-opinion capabilities only; `auto` picks a
provider offering the capability; explicit target never replaced; a Codex session
generating an image through Codex; label persisted and replayed; a record without
the label; discovery performs no model or quota request; an unknown id.

Live: from a Codex session on Hawk, one real `image.generate` call, with its
artifact verified, then the same call refused after the session turns it off.

## Later slices

Each needs its own contract:

- **Declared capabilities.** Manifest-level capability declarations, per preset,
  so a model exposes what it can do without a Cowboy release. Requires a Provider
  SDK and signed plugin release, and opening `Purpose` with readers shipped before
  writers on Controller, Machine and worker.
- **The envelope.** `{capability, inputs, implementation}` replacing `purpose`.
  Only worth doing with declared capabilities.
- **P5–P7** of [background tasks](background-tasks.md#8-later-phases-each-needs-its-own-contract-none-blocks-phase-1):
  trust, resource control, discovery.
