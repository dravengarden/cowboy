# Machine durable-state diagnostics

A read-only, content-free report of the durable datasets a Machine owns, so an
operator or an AI assistant can see their state without reading logs and state files
by hand. It extends the diagnostics row of the
[completion ledger](plugin-refactor-completion.md) to the new finite domains. It
changes no dataset and grants nothing.

## Surface

`GET /api/machines/{id}/durable-state` on the Controller, for a connected Machine.
Authentication is the product session or the Operator CLI (the same class as the
other observability routes), not the public, credential-free
`deployment-health`. Reply (schema 1):

```json
{"schema":1,
 "deletionJournal":{"writerEnabled":true,"deletedSessions":7},
 "sessionIncarnations":{"writerEnabled":true,"lineages":6},
 "cleanupContinuations":{"pending":0},
 "controllerObserved":{"sessions":8,"withLineage":8}}
```

`controllerObserved` is added by the Controller itself, not reported by the Machine:
how many of that Machine's Sessions the Controller knows, and for how many it holds a
Machine-reported [lineage](plugin-session-incarnation-carriage.md). It is counts only,
read from the live Hub, so it is process-local and starts at `withLineage: 0` after a
Controller restart until the Machine's snapshots arrive. A Machine with no admitted
incarnation writer leaves it at 0, which is how the carriage is told apart from the
process-local fence. The Machine's own part of the reply is unchanged and still decoded
against the closed schema.

A dataset the Machine does not hold is `null` (for example the cleanup namespace on a
build without an admitted deletion writer). The report carries counts and writer flags
only: no Session ID, lineage value, path or record.

## How it works

The Machine answers the generic `adapter_request` kind `durable-state` with an empty
object as its only accepted request. Its handler reads a `DurableStateView`, which holds
the broker **weakly** (it never keeps one alive and is empty until a broker attaches),
and returns the counts. The Controller decodes the reply against a closed schema, refusing
unknown fields, another schema and counts above one million, and never proxies free-form
JSON. A Machine that predates the report answers its request with an error, surfaced as
`502`; a Machine that is not connected is `404`.

The shared schema lives in `src/durable_state.rs`, outside the Machine protocol and
runtime wire files, so adding it does not change the retained worker interface that the
host-only Machine releases compare byte for byte.

## Limits

Each dataset is read separately: this is not an atomic snapshot across datasets, and a
count can move between two reads. It reports what the Machine holds in memory for its
open namespaces, not an audit of the files, and says nothing about the root-owned floors.
It does not report incarnation values, so it cannot show a lineage change. There is no
Web panel yet. `cowboy operator durable-state --machine <id>` reads it through the private
Operator endpoint, which reuses the same handler; it is live on Hawk
([release](releases/durable-state-diagnostics-2026-10-06.md)).
