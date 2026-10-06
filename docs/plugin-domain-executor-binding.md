# Binding resolved results to domain executors — the rule, and why nothing generic

Closes the analysis of the "link exact resolved results to finite domain executors; no
serialized authorization" part of the P0 exit in the
[completion ledger](plugin-refactor-completion.md). It adds no code. Its finding is that the
mechanism already exists in the one domain whose executor needs it, and that a generic version
is what the ledger forbids.

## The rule, as the code states it

`src/composition/mod.rs`: the structural checker reads no Catalog, credentials, policy or live
state; its input releases and ports are **untrusted claims**; a checked proposal is not an
authorized plan or a verified release and **no installer or executor accepts it**. Runtime
integration must resolve verified packages, authority, fences and leases itself.

`src/composition/telemetry.rs` is the one finite domain that does, and shows the shape:

- `ResolvedBinding` and `ResolvedExport` have private fields and **no `Clone` and no serde**, so a
  checked report or a deserialized receipt cannot become an executable port.
- They are built only by `resolve(catalog, control, step)` from live inputs: the Catalog, the
  Machine's authenticated connection (a `ConnectionToken`) and the exact step. Any missing or
  changed input fails closed.
- The executor holds the value and calls `current()` before each effect, which re-reads the
  connection, the installation lease and the Catalog release. The first failure sets an `ended`
  flag that is never cleared, so an ended result cannot revive.
- The module says plainly that these results are **not grants**: Operator or standing policy, the
  time budget, durable namespace CAS and Machine-private policy remain independently required by
  the caller.

## Inventory (what was checked against code, and what was not)

| Domain | Executor-side binding | Checked how |
| --- | --- | --- |
| Managed telemetry binding and export | `ResolvedBinding` / `ResolvedExport`, rechecked per effect | read in code, with tests that resolution fails closed |
| Session read observations | `SessionCodeScope` and Workspace scopes compared for equality at the point of use; a lineage change renews the Session lifetime on the unshipped carriage branch | read in code |
| Plugin install and uninstall | original authenticated connection, execution leases and the accepted Catalog, per the install and execution-lease records | from the documents, **not re-read in code for this note** |
| Session lineage | owner-minted value with no Controller constructor | read in code |

No other domain has an executor that takes a composition result.

## Why no generic mechanism

Every executor needs inputs only its domain can resolve (which connection, which lease, which
policy epoch). A generic "resolved graph" type would have to carry all of them, grow with every
domain, and be the unconstrained executor the design refuses ("a read-only composition is not an
authorized generic DAG", "do not add a parallel Plugin lifecycle"). The mechanism that exists is a
rule applied per domain, and it is enough while each domain brings its own `Resolved*` type.

## Checklist for a new domain executor

1. A `Resolved<Domain>` with private fields and no `Clone`, `Serialize` or `Deserialize`.
2. A `resolve(...)` that takes only live inputs (Catalog, authenticated connection, current
   fences or leases) plus the exact step, and fails closed on every missing or changed input.
3. A monotone `ended` state and a `current()` rechecked before each effect, never cached across
   an await that can outlive the input.
4. No path from a checked report, composition proposal or receipt to the executor.
5. The caller still supplies authority, budget and any durable CAS: the resolved value is not a
   grant.
6. Tests that every input, removed or changed after resolution, ends the value, and that the old
   value cannot be reused after the input returns.

## Limits

This is a reading of the code and documents, not an acceptance. It does not cover domains that do
not exist yet, and the install and uninstall row rests on the records, not on a fresh code review.
The ledger exit stays open for the domains that have no executor-side binding; the rule above is
their acceptance contract.
