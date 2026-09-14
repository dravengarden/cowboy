# Connected installation acceptance

This gate executes the core installer with two real immutable release processes.
It complements the Service 168-case and Machine 72-case populated reader gates;
none of the three substitutes for actual host role capture or activation.

```sh
nix develop -c just plugin-install-connected-conformance /absolute/matrix.json /absolute/new-receipt.json
```

Use the existing schema-one Controller/Machine matrix, with `active`, `rollback`
and `cold` immutable release roots in each lane. `rollback` means the **next
transaction's recovery reader**, not an older transaction's historical
`previousRelease`. The active pair must support fresh protocol-19 installations.
Recovery and cold readers may deliberately pause fresh installation admission.
The harness requires clean committed source, hashes the actual ELF chains, runs
in an isolated loopback-only network namespace and writes a private create-only
bounded receipt.

## Real writes, independent reads

Each flow creates a disposable signed Victoria release, empty tracked Machine
slot, genuinely enrolled Machine identity and random password-authenticated
Operator. No production credentials, state, policy, destination or package are
used. Installation does not enable telemetry export. The relay permits only
the authenticated protocol-19 handshake, exact empty-worker generation setup,
inventory and the flow's exact target/step/receipt. It forwards original bytes,
never fabricates replies and refuses auth/session/export/legacy commands.

| Writer flow | Service evidence before reader startup | Machine evidence |
| --- | --- | --- |
| Install, then reinstall identical bytes with another ID | Two completed operations, full exact receipts | Two Applied receipts, distinct incarnations; second CAS targets the first |
| Drop Applied receipt, retain connection | NeedsAttention / UnknownMachineOutcome after the real 90-second deadline | Applied retained |
| Disconnect after Applied, before reply delivery | NeedsAttention / UnknownMachineOutcome | Applied retained; reconnect does not renew authority |
| Disconnect before step delivery | NeedsAttention / UnknownMachineOutcome | No attempt namespace or installation effect |
| Kill Controller after Applied, before reply delivery | Installing with no Machine receipt | Applied retained |

After both writer processes stop, each of the nine reader pairs independently
copies the same bounded private fixture, including its stopped SQLite/WAL state,
and opens that state twice: **45 checks, 90 cold reads**. No reader receives an
already normalized copy from another reader. The crashed Service operation may
become NeedsAttention / Interrupted on the first open; its original intent and
all Machine evidence remain unchanged. Every other operation must be identical
to the writer's evidence, including exact checksums and timestamps. The second
open must preserve the first open's result exactly.

History must expose only the closed v2 projection, never a target, actor, plan,
deadline or grant. Reusing the original real fixture login cookie after restart
does not authorize replay: same-ID requests return historical 409 (or paused
reader 503), and changed-input IDs are refused. No extra target query, step,
receipt fallback, write or inferred compensation is allowed. Reader restart
must not release an unknown slot fence.

## Limits

This is not production Operator acceptance, Agent authentication projection,
PostgreSQL process startup, native worker generation restoration, physical
power-loss testing, independently approved compensation or completion of the
Plugin refactor. Real production installation and managed telemetry cutover
remain separately authorized operations. A passing candidate receipt alone
does not prove the deployed host is running those artifacts.
