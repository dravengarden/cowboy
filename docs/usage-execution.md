# Account usage execution

Settings → About → Usage lets an operator choose a **Usage query Machine**
for each account. This is a Service-wide, durable selection. It does not move
Agent sessions, change their model routing, or export Provider credentials.

Automatic preserves the existing selection: choose an online Machine with an
active installation matching a signed usage host's exact Plugin version and
generation digest. A named Machine restricts that same eligibility check to
the selected identity. An offline or incompatible pinned Machine produces an
unavailable result; Cowboy never silently falls back to another Machine.
Existing cached usage remains historical data, not evidence of a fresh query.

Controller-host `--provider-runtime-machine PROVIDER=MACHINE` restrictions
intersect this selection by the exact Plugin/Provider identity, independently
of the usage account key. An explicit conflicting choice or an unavailable
allowed Machine produces an unavailable result. Automatic cannot bypass the
restriction, and a restricted account cannot fall back to a Controller-local
bootstrap command. The preference remains editable for recovery after a host
policy change, without granting permission to a currently denied route.

The selection applies to the account's collection, activity decoration and
supported usage-reset operations. Session-reported usage remains associated
with the session and does not become account-plan usage.

## Operator CLI

Use a CLI matching the Controller release through its authorized local Operator
socket. Read configuration and current eligibility:

```sh
cowboy operator usage-executor
```

Set one account, then request a fresh sample:

```sh
cowboy operator usage-executor --provider anthropic --machine ovh
cowboy operator usage --refresh anthropic
```

Use `--machine automatic` to remove an explicit selection. Account keys come
from the response's `providers` map; Machine IDs come from `machines`.
`machine_id` is the configured selection and `selected_machine_id` is the
currently eligible execution target, not proof that the Provider query succeeded.

The Web API uses `GET /api/usage/executors` and operator-authorized
`PUT /api/usage/{account}/executor` with `{"machine_id":"ovh"}` (or `null` for
Automatic). Viewers can inspect settings but cannot change them. The dedicated
`usage_execution_machines` table belongs to usage, independently of authentication
settings. Writes serialize with active refreshes; a slow collector can delay a
save. Successful changes clear the previous route's refresh cooldown.

## Verification

The routing test covers explicit selection over a newer alternative and no
fallback when a pin is unavailable. SQLite and isolated PostgreSQL contracts
cover durable restoration, clearing the pin, and rejecting unknown accounts or
Machines. API client tests cover reads, updates, Automatic and permission errors.
