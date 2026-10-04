---
type: guide
description: App-owned Cowboy Plugin installation through Cardea human-reviewed operations.
---

# Cardea operations for Cowboy

Cowboy's first generic Cardea action is `plugin.install`. Cardea handles device
identity, the approved action catalog, exact-plan review, and single-use claims.
Cowboy owns account mapping, current operator permissions, trusted release
selection, Machine compatibility, installation preconditions, and execution.
A Cardea approval cannot grant a Cowboy role or override a local refusal.
If a grant already maps to a Cowboy device, its local revocation and account
ownership apply to generic operations too. The adapter does not require an
additional Cowboy bearer login for a separately approved Cardea-only profile.

This adapter is disabled by default. Enable `COWBOY_CARDEA_OPERATIONS_ENABLED=true`
only after the Cardea generic-operation release and this Controller release are
available, the exact catalog has received human approval, and end-to-end
acceptance has passed. Product authentication must also be enabled; the local
authentication-disabled owner shortcut cannot activate this adapter. Existing
browser administration and explicitly delegated host Operator access remain
separate authorities. Other Cowboy actions have not yet migrated to Cardea.

## Catalog registration

The reserved Cardea Authentication Provider supplies the trusted issuer,
Ed25519 backend client, allowed subject, and local account mapping. Never copy
its private key into an AI profile. Use the application's existing registered
client and a separately enrolled device profile.

Inspect `cardea --version`, `cardea --help`, and `cardea --agent agent guide`
first. A device-only binary does not support this workflow. The release-matched
`cardea-authorization` skill ships with the native Cardea binary.

Generate and save a change ID with `cardea --agent agent id`. Generate a proposal
using that ID and the current catalog revision (zero only for a new catalog):

```bash
cowboy cardea catalog --application cowboy-production \
  --origin https://cowboy.example --change-id SAVED_CHANGE_ID \
  --expected-revision CURRENT_REVISION > cowboy-catalog.json
cardea --agent --profile cowboy change propose --input-file cowboy-catalog.json
```

The origin must be the Controller's exact configured public origin. Replace
the example origin and revision with authoritative deployment values. Inspect
the proposal, then let the person review its returned Cardea URL. CLI success
does not mean catalog approval. There is no CLI command for human approval.

## AI operation workflow

Discover the approved input contract through
`cardea --agent --profile cowboy action view plugin.install`. Create an input
file containing only the intended exact Machine, Plugin, signed version, and
artifact digest:

```json
{"machine":"hawk","plugin":"PLUGIN","version":"SIGNED_VERSION","digest":"sha256:EXACT_ARTIFACT_DIGEST"}
```

Do not provide `target` or `envelope_digest`; Cowboy fills these from the live
Machine installation observation and trusted signed release. Service-managed
Machine components are refused; they retain their host-owned maintenance path.

```bash
cardea --agent --profile cowboy operation prepare --action plugin.install \
  --input-file install.json --idempotency-key SAVED_REQUEST_ID
cardea --agent --profile cowboy operation request OPERATION_ID
cardea --agent --profile cowboy operation wait OPERATION_ID --timeout 30
cardea --agent --profile cowboy operation execute OPERATION_ID --claim-id SAVED_CLAIM_ID
cardea --agent --profile cowboy operation receipt OPERATION_ID
```

Generate and persist the request and claim IDs before their respective
mutations. Inspect the prepared target, release, deadline, policy revision, and
plan digest before requesting review. The human approves the immutable plan in
Cardea. After approval, read authoritative state; do not infer it from chat.

Changing a Machine, Plugin, version, artifact, account mapping, permission
policy, or installation precondition requires a fresh plan and review. Denial
stops automatic retries. Do not substitute host delegation, browser credential
copying, direct installation APIs, or SSH when Cardea refuses an operation.

## Execution and recovery boundaries

The Controller consumes verified SDK authority only for the original
installation purpose. It uses the existing finite installer, signed-release
checks, generation/session protections, and Machine compare-and-swap contract.
Installation dispatch rechecks the reviewed target, current authority, freeze,
and claim start deadline. Pre-install Provider authentication synchronization
also checks this deadline and current policy immediately before dispatch.
Post-install authentication synchronization retains the original identity
budget and current policy. Captured permission observations are permanently
invalidated by revocation; granting a role again does not revive old authority.
Already-running work remains bounded by the original identity lifetime and
existing five-minute installer budget. The approval plan lasts ten minutes;
this does not extend the exchanged identity or claim start deadline.

Prepared plans, original claims, dispatch reservations, and confirmed receipts
are durable in both PostgreSQL and SQLite and are retained during database
copy. Once dispatch is reserved, an interrupted HTTP request or Controller
restart never admits another dispatch for that operation. An unknown outcome
stays reserved; receipt reconciliation reads the original native installer
journal. Confirmed receipts are immutable. `AuthenticationPending` reports an
applied installation whose Provider authentication still requires completion.

After a lost response, query the same operation receipt at most three times,
with ten seconds between queries, then report unresolved state and resume
later. Do not execute again or allocate another ID to resolve uncertainty.
An approval, consumed claim, or HTTP success alone does not prove installation.

Protocol endpoints use the standard `/cardea/v1/` adapter routes, bounded JSON
documents, no-store responses, eight concurrent admissions, and 120 admission
requests per minute. The retained journal has a hard 4096-operation cap. When
full, new preparation fails closed. Operators must plan retention maintenance;
do not delete uncertain reservations to make space or permit replay.

## Acceptance

Unit conformance exercises the real Cardea SDK and Cowboy authority boundaries,
including role revoke/regrant, disabled accounts, freeze, stale target,
purpose substitution, identity expiry, claim expiry, and durable single-owner
reservations. Transport tests cover signed backend authentication, strict JSON,
bounded responses, and redirect refusal. These fixtures are not production
device login, human approval, or an actual Machine installation receipt.

Before enabling production, exercise real catalog review, plan approval and
denial, native installation receipts, and lost-response reconciliation across
the deployed Cardea and Cowboy services. Preserve active sessions and the
independently retained Machine/worker release. Codex and Claude share the same
CLI/SDK protocol; their live task state remains runtime-owned.
