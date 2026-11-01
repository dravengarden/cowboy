# Call capabilities

Status: [background tasks](background-tasks.md) Phase 4, first
slice, implemented; revised after the Astra Max design review (11 findings, none rejected; see
[Decisions](#decisions)).

## What this slice is

A person chooses per session whether agents may use images, and a Codex session
may generate images through Codex itself. A model sees that state and decides.

It is **policy over the fixed Provider profiles**. It does not deliver a model or
preset declaring its own capabilities, or an implementation bound at admission;
those are [later slices](#later-slices). Calls stays the common lifecycle
([managed native capabilities](managed-native-capabilities.md)).

## The gap

- Policy is per agent. A session can allow Codex but cannot refuse its images.
- `same_agent` exists so a second opinion comes from another family, yet it also
  stops a Codex session reaching the only Provider that offers images.
- `calls.capabilities()` reports what Providers offer, not what this session may
  use, so a model finds out by being refused.

Neither Codex nor Claude Code routes a task to another Provider's account on
another Machine, so this policy stays Cowboy's until a native registry offers the
same scoped routing. Claude is shared: it is all Controller-side.

## Switches follow enforceable boundaries

A switch must be enforceable by the runtime. `image.generate` and `image.edit`
share the `ImageV1` profile, and `review`, `design_review` and `analysis` share
`ReadOnlyV1`: a request for one can do the other, so finer switches would promise
what nothing enforces. There is therefore one switch per profile, and the existing
`calls.enabled` already governs the read-only profile.

| Field of `calls` | Default | Meaning |
|---|---|---|
| `images` | on | Image calls may run at all |
| `self_images` | off | Image calls may also use the caller's own family |

`images` on keeps today's behavior. `self_images` is new authority, so it is off:
turning it on is the person's choice and spends that family's account.

## Policy storage

Both are fields of `CallsPolicy` and `CallsOverride`, so one record and one
in-memory update hold a session's whole policy. They default on read.

`AgentTools` and `SessionTools` parse strictly, so an older Controller cannot read
a record holding them, and would silently drop that session's whole override.
That is safe here for two reasons, and only together:

- The Controller ships before any writer. Until the Web ships nothing stores these
  fields, so the release's own automatic rollback finds nothing it cannot read.
- The activator refuses a release that does not descend from the active revision,
  so a later rollback is a descendant revert. **A revert must keep the readers.**

## Admission

Order in `start`, after the existing replay of a known `request_id` (which keeps
returning the original call whatever the policy says now):

1. An image request with `images` off: `capability_disabled`,
   `admission: not_submitted`. This precedes approvals, and neither Allow once nor
   Allow for session can override it: a person changes it in Tools. It is
   evaluated on every submission, so a resubmission sees a later change.
2. Selection, through one resolver used by admission, approvals and both forms of
   discovery. For an image request with `self_images` on, the caller's family is
   eligible:
   - explicitly, or by continuing that image conversation;
   - for `auto`, as the last candidate, only when `calls.default` is `auto`. A fixed
     default stays fixed.
   Everything else keeps `same_agent`. `calls.enabled` and explicit target
   restrictions still apply, and a self-call with calls off is refused without a
   prompt, because Allow once cannot grant a family target.

A self-call is not recursion: the image profile has no Cowboy tools. It counts
against the same parent session limits, which remain soft, advisory count-then-insert
checks; concurrent submissions can pass them together. Atomic reservation is P6.

Cancelling a running call is unaffected by turning `images` off.

## Agents cannot widen themselves

`override_widens` and `fresh_session_widens` both treat turning on `images` or
`self_images` as widening, so an agent can only narrow, including through
`sessions.duplicate` with `inherit_settings: false`, which already goes through
`fresh_session_widens`.

Continuation copies a sparse override. A denial inherited from the source's Owner
defaults would be lost when the destination's defaults differ, so inheritance
writes the source's effective `images: false` and `self_images: false` into the
copy.

## Discovery

`calls.capabilities()` gains:

```json
{"capabilities": [
  {"id": "read_only", "tasks": ["review", "design_review", "analysis"],
   "enabled": true, "providers": ["codex"]},
  {"id": "image", "tasks": ["image.generate", "image.edit"],
   "enabled": true, "self": false, "providers": ["codex"]}
]}
```

`providers` lists targets this session may use for that profile whose installed
signed package offers it. The scoped `calls.capabilities({capability})` reports
`capability_disabled` and uses the same resolver. Discovery reads installed state:
no model request, no quota refresh. The model decides; nothing here routes a task
or adds to a resident prompt.

## Records

Nothing changes in the record or the request. The capability is read from
`purpose`. In particular no label is added: labels are part of the request digest
and of the record's match against its request.

## Product

Tools (session) and Settings → Agent tools gain **Images** and, under it, **Use this
agent's own account**. A session override shows the way back to the default, like
the other fields.

## Rollout

Controller, confirm health, then Web. No Machine release and no migration.

An already open older Web sends `PUT .../tools` without these fields, which clears
them from that session's override. That can only happen by a person saving Tools
in a stale tab, and a Web update prompt covers it.

## Acceptance

Automated: defaults keep behavior; session over Owner over built-in; old records
without the fields; `images` off refuses before approvals and before any record,
and Allow once cannot override it; a known `request_id` still returns its original
call after the policy changes, and a conflicting one is still a conflict; `same_agent`
for read-only tasks always; explicit self image, `auto` last-candidate self image, a
fixed default not replaced, continuation of a self image; `calls.enabled` off refusing
a self image; agent-submitted enable refused and disable accepted; a person's enable
accepted; fresh session and non-inheriting duplicate widening; an inherited denial
kept across a Provider change; both discovery forms agree, with no model or quota
request.

Live: from a Codex session on Hawk with `self_images` on, one real `image.generate`
call with its artifact verified; then a new `request_id` refused after turning
`images` off.

The live run on 2026-11-01 from a Claude session covered discovery, a real image
through Codex, narrowing, a known `request_id` replaying its original call, and an
agent's enable refused with `tools_widening_denied`. It found that every refused
call reached MCP as `outcome_unknown`, because the reply lost the schema the
gateway requires; that was fixed. `self_images` needs a person to turn it on, so
the Codex self-call above is outstanding.

## Later slices

Each needs its own contract:

- **Declared capabilities.** A signed preset listing the capabilities its model
  serves, so a model exposes what it can do. Today Provider-level differences are
  already the signed `managed_profiles`, and the models of one Provider offer the
  same capabilities, so a declaration would change nothing. It is also not a small
  change: Provider SDK source sits in every plugin's component closure, so editing
  it means a component release and new signed versions of all plugins, and a
  manifest using the field needs Machines and workers built from that SDK. A real
  difference between models is what justifies that release.
- **New capability kinds.** A capability outside the two profiles needs its own
  signed profile, which the Machine and worker must read, and opening `Purpose`
  with readers shipped before writers.
- **Enforced task boundaries.** Separate generate from edit, or review from
  analysis, by carrying permitted native actions through the worker and adapter.
- **The envelope and admission binding.** `{capability, inputs, implementation}`,
  with the implementation pinned at admission rather than at claim.
- **P5–P7** of [background tasks](background-tasks.md#8-later-phases-each-needs-its-own-contract-none-blocks-phase-1):
  trust, resource control, discovery.

## Decisions

From the design review (`call-dc3a839502839f73567ddd89`):

| # | Finding | Resolution |
|---|---|---|
| 1 | Creation escapes widening; sparse copies lose denials | Both widening checks cover the switches; inheritance writes effective denials |
| 2 | Switches not enforced between generate and edit | One switch per runtime profile; finer boundaries are a later slice |
| 3 | A capability label breaks ledger invariants | No label; read from `purpose` |
| 4 | Separate keys allow partial updates | Fields of the single policy record |
| 5 | Rollback removes denials | Controller before writers; a descendant revert keeps the readers |
| 6 | Approval could override a disabled capability | The denial precedes approvals on every submission |
| 7 | Auto must keep fixed defaults | Self target only when `calls.default` is `auto` |
| 8 | Family exception touches more than admission | One resolver for selection, approvals and discovery |
| 9 | Live test conflicted with replay | New `request_id`; replay covered separately |
| 10 | Target budgets do not exist | Stated as shared, advisory parent limits |
| 11 | Digest is not bound at admission | Stated as not delivered; a later slice |
