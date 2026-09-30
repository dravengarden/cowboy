# Nullable Catalog digest convergence repair

Hawk's `cowboy-plugin-converge.service` failed before planning installations
with `reading the Catalog: invalid type: null, expected a string`. The actual
Catalog contained an unbound Plugin entry with `artifact_digest: null`.
`PluginCatalogEntry` correctly serializes its optional digest this way, but the
Operator CLI's `CatalogRelease` required a string. Serde's field default handles
an absent field, not an explicit null.

The CLI now normalizes absent/null digests to its existing empty-digest state.
Ready-release selection still rejects missing or empty digests, even if the
entry claims to be ready. Non-string values remain invalid. This changes no
signature, installation, authorization, Machine, or credential behavior.

All 19 convergence tests passed, including a regression covering null, absent,
and empty digests for both ready and unbound entries, preservation of a valid
ready release, and rejection of a numeric digest. The freshly built official
CLI also completed a read-only `operator converge --machine hawk` against the
real Service: `applied=false`, one Machine, zero unreachable Machines. No
installation was submitted by that dry run.

The complete pinned-shell `just check-compact` passed, including lint, frontend,
PostgreSQL, Plugin conformance, and release builds. Native diff review found no
actionable defect. The immutable release and production activation are recorded
below. CLI dry-run validation alone did not repair the installed timer's
executable; that required the component activation. Hawk's unrelated LiveView backup
failure also remains a separate blocker for the Columbus host transaction.

## Production activation

Source `14c77e88f56d76efe84e0f6d422eddbf231e5fc1` was published to main and
built cleanly as Controller release
`/nix/store/57czafnjavsxpkcmv0s9hhmdnk6jiiyr-cowboy-controller-release`.
The immutable release's CLI passed the same real-Service dry run before
activation. The owning component activator completed transaction
`1790729939408406762-14c77e88f56d` at `2026-09-30T00:59:11.829736124Z` with
`outcome=succeeded`. Its receipt remains at
`/var/lib/hawk-component-deployments/cowboy-controller/current.json`.

Postflight verified the running Controller executable resolves to this exact
release, local and ordinary private-origin HTTPS health checks return success,
and the installed component-profile CLI completes Catalog convergence planning
with `applied=false` and no unreachable Machine. All 19 pre-existing resident
Machine/worker units retained their PID, monotonic start timestamp, and restart
count. Web stayed at `ff0260393edeb134b45ed3efdeeca6dd66e74de6`; Machine stayed
at `a225d618a9bf60fd71d2a2a8cbab73c70711914a`.

No manual `converge --apply` was run: the plan contained six upgrades, which
belong to the existing signed Plugin lifecycle and active-session lease gates.
The timer's old failed status is historical until its next genuine execution;
it was not reset to manufacture a successful run. This receipt proves the
Catalog reader repair and bounded Controller release continuity, not permanent
OVH Machine enrollment or the outstanding interactive Grok acceptance.
