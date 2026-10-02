# Claude recommendations and default, 2026-10-02

Claude Code Plugin **3.3.1** is published and installed on OVH. It adds
Sonnet High as the first recommendation and the new-session default, restores
Opus High and names the model-default effort card Opus Recommended. Existing
Sonnet Medium and both Fable presets remain. The exact source and installation
are recorded in the [production receipt](claude-presets-2026-10-02.json).

| Recommended card | Model value | Effort | New-session default |
| --- | --- | --- | --- |
| Sonnet · High | `sonnet` | `high` | Yes |
| Sonnet · Medium | `sonnet` | `medium` | No |
| Opus · High | `opus` | `high` | No |
| Opus · Recommended | `opus` | `default` | No |
| Fable · High | `claude-fable-5-1[1m]` | `high` | No |
| Fable · Max | `claude-fable-5-1[1m]` | `max` | No |

The missing Opus cards were a configuration mismatch: the signed presets used
`opus[1m]`, while the actual OVH session's ACP model options advertise `opus`.
The generic compatibility filter therefore correctly hid both old cards.
The fix changes the Plugin configuration, keeping the filter intact. The current
[upstream model configuration](https://code.claude.com/docs/en/model-config)
documents the aliases; the actual persisted live ACP options were inspected
read-only to establish this account's available values.

`configuration_presets` supplies ordering and the default badge;
`runtime.behavior.default_preferences` now selects `sonnet` and `high` for new
sessions, retaining `bypassPermissions`. Existing sessions can adopt the newer
signed presentation through the normal Catalog refresh while retaining their
selected model/effort and executable generation. Opus Recommended means Opus's
native default effort; it is not the default Cowboy preset.

## Acceptance and activation

- `plugin-check`, `provider-check`, the complete `just check`, and 40 focused
  preset/Provider UI checks passed. Projection against the actual session's
  model/effort options accepts all six cards and exactly one Sonnet High default.
- All four runtime archives are byte-identical to 3.3.0 and reused through the
  owned runtime binder. Native CLI 2.1.287, ACP 0.84.0 and Mods source are unchanged.
  Actual old/new detached workers passed startup, coexistence and descendant drain.
- Exact signed bytes passed active/recovery/cold and previous Controller Catalog
  readers. Five public URLs matched their SHA-256 digests. The refreshed Catalog
  reports the exact 3.3.1 release as ready.
- Operation `ovh-claude-3-3-1-sonnet-high-20261002` exceeded its initial
  90-second observation deadline. Same-ID reconciliation obtained its original
  Applied receipt and completed authentication finalization without repeating
  installation. No slot fence remains.
- OVH inventory reports 3.3.1 active with current auth replica/materialization.
  Controller, OVH Machine and both existing worker PIDs/start times are unchanged;
  health is normal. No core component or Web release was activated.

Release digest:
`sha256:b6943cb0516de6d00ec637b2ad5ede1b0914d09217e8e20f8b850da190200c56`.
Installation revision:
`installation-9b882c5de39b3ed5e5c19554119fe921e01cf9ae78d48078329cfd2bee337a4d`.

Verification did not send paid model inference or drive the physical iPhone UI.
The Linux worker gate used isolated fixture authentication. The first two worker
invocations supplied an incorrect artifact-root directory and failed before
runtime startup; the accepted invocation used the published Catalog root.
