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
below when completed. Until activation, a successful new CLI dry run does not
repair the installed timer's executable. Hawk's unrelated LiveView backup
failure also remains a separate blocker for the Columbus host transaction.
