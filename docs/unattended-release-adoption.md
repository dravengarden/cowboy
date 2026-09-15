# Unattended release adoption

Status: first slice implemented, 2026-09-15. The Controller now adopts published
Catalog bytes without an operator session. Machine installation still requires a
person-bound credential.

Layers 2 and 3 are **accepted and deferred**: the design below is agreed, no
work is scheduled, and until one of them lands every installation continues to
need a role >= Operator personal token or an admin cookie. Neither layer is a
prerequisite for the other's value, but the rollout order at the end of this
document still holds when they are picked up.

## The gap this closes

Publication never needed authentication. `just plugin-publish` signs a release
with an Ed25519 publisher key and hard-links immutable bytes into a Catalog
directory; `load_catalog_root` verifies every package against
`trusted-publishers/<publisher>.pub` before granting any identity. The Catalog
boundary is anchored in signatures, not sessions.

Yet a published version stayed invisible until a human opened the admin surface,
because the only paths that reloaded it were process start and
`POST /api/plugins/catalog/refresh`, which requires an admin **cookie** and has
no bearer-token branch. A release could be fully signed, verified and immutable
on disk and still wait on someone's browser.

That is an interface gap, not a security property. Noticing a signed release is
not an authorization decision.

## Principle

Move the authority for unattended delivery from _who is logged in_ to _what is
signed_ and _what policy declares_. Every layer below keeps a verifiable
authority and a recorded actor; none of them removes one.

An unauthenticated installation endpoint is explicitly rejected as a design: it
would let anything that can reach the Controller place arbitrary executables on
every enrolled Machine. The goal is no _human interaction_, not no _authority_.

## Layer 1 — Catalog watch (implemented)

`src/plugin_catalog/watch.rs` observes each selected Catalog root and re-runs
the same `refresh_with_runtime` + `ProviderCatalog::refresh_external` pair that
startup and the admin endpoint already run. It introduces no endpoint, no
credential and no new trust boundary: writing to the Catalog directory was
already the publication boundary.

The publisher's `copyImmutable` stages into a temporary file and hard-links it
into place; the release envelope is linked **last** as the commit marker. The
trusted reader ignores uncommitted packages and verifies supported committed
releases before accepting a candidate. Notifications, metadata and directory
write access do not replace that verification. A corrupt candidate can still
fail; publication of several releases is not one atomic transaction.

- Catalog roots and their existing `trusted-publishers` directories have
  separate nonrecursive watches. The Provider compatibility root is observed
  only when the existing Provider reader already selected it.
- Publication bursts settle after 750 ms of quiet, capped at three seconds so a
  sustained stream cannot indefinitely postpone a read.
- A read-only metadata scan runs at startup and every 30 seconds. It covers
  absent or recreated roots, missed events, unavailable watches and backend
  errors without creating paths or requiring another directory write. It does
  not re-arm native watches after replacement; the timer remains the fallback.
- The scan samples only named package, release-envelope, host-bundle and public
  publisher-key metadata, including link and target identity. It does not open
  file contents, recurse into artifacts/private state, or hash credentials.
  Directory access/write times and unrelated files do not trigger host rebuilds.
- The shared per-scan budget is 8,192 directory entries (including ignored
  names) and 1 MiB of path/name bytes. Exceeding it or failing a scan preserves
  the accepted runtime and retries at a later observation. These bound observer
  work, not OS I/O latency or all trusted-reader resource use. They are not new
  universal Catalog size limits.
- An unchanged accepted metadata hint skips runtime reconstruction. Hints are
  process-local scheduling aids, never signed identities, continuity leases or
  installation grants.
- `POST /api/plugins/catalog/refresh` remains as an idempotent manual fallback.

### Lifetime and partial failures

The Controller owns the observer task. Service shutdown, loss of its signal
sender, or dropping the observer stops new probes, settling and retry admission.
A probe already in progress completes read-only and cannot initiate refresh
after stop. An already-started refresh is **drained**, not aborted: generation
staging or migrations may have begun. The normal shutdown path joins it before
storage teardown. This can delay shutdown by the ordinary refresh duration; it
does not supply hard-kill recovery, compensation or rollback of physical state.

A failed Plugin Catalog/runtime candidate preserves the previous visible Plugin
snapshot. Provider projection happens **after** that snapshot commits. A legacy
Provider failure therefore leaves the new Plugin snapshot and previous Provider
projection; this is not an atomic two-Catalog update or a rollback. Error
context identifies the failing stage, with retries after 2, 15 and 60 seconds.
After that batch, periodic source checks can retry even without another event.
Stop prevents further retries but still drains any attempt already admitted.

Automated coverage includes real notification regressions, bounded source
sampling, paused-time ownership/retry tests and actual signed Catalog fixtures:
new publication, invalid signatures with original leases retained, and Provider
failure/recovery after Catalog commit. These are not production Plugin
installation, native-generation acceptance or host-policy authorization.

## Layer 2 — Host-managed deployment policy (not implemented)

`cowboy.service` already renders `plugin-hosts.json` through an `ExecStartPre`
and trusts it with no session. A deployment policy follows that precedent: a
root-owned declarative file naming, per Machine, which Plugins track which
publisher.

```json
{
  "schema": 1,
  "publisher": "cowboy-first-party",
  "activate_when": "provider_idle",
  "max_version_jump": "minor",
  "machines": {
    "hawk": { "track": ["claude-code", "codex", "grok"] },
    "falcon": { "track": ["*"] },
    "macbook-air": { "track": [] }
  }
}
```

When the Catalog gains a newer ready release signed by the named publisher, the
Controller creates the installation operation itself. There is no request to
authorize because there is no request; the authority is the declared policy.

Three constraints are part of the design, not options:

- `activate_when: "provider_idle"` queues instead of activating while that
  Provider has a live turn. Staging beside the active generation, atomic
  activation and the rollback link already exist; the policy only supplies an
  activation condition. This is how the rule that a Machine release is an
  explicit maintenance boundary survives automation.
- `publisher` is an allowlist. A release signed by anyone else may sit in the
  Catalog and will never be adopted automatically.
- `max_version_jump` forces large jumps to stay manual. A Machine several minor
  versions behind is exactly the case that wants a person watching.

## Layer 3 — Signed deployment intent (not implemented)

Layers 1 and 2 cover "follow the current release". A targeted deployment — one
Plugin, one Machine, initiated by CI or an agent — needs a credential that is
not a person.

Add a `Credential::DeploymentIntent { key_id, intent_digest }` variant to
`src/server/operator_approval.rs`. The request carries a canonical intent
document and a detached Ed25519 signature over it:

```
(service_id, machine_id, plugin_id, version, artifact_digest,
 operation_id, not_before, not_after)
```

Verification uses a **separate** `trusted-deployers/<key_id>.pub` keyring.
Publishing authority and deployment authority stay distinct: holding the
publisher key must not imply the right to place bytes on a Machine.

The document carries `operation_id`, so it composes with the existing
duplicate-observation rule — a repeated identity is observation only and never
creates a replacement grant. `not_before` / `not_after` bound replay.

## Invariants that do not change

- The Machine re-verifies the signature, stages beside the active generation,
  probes every executable, activates atomically and retains a rollback link.
  That probe is where a cross-built target first proves it can execute.
- `resolve_verified_exact(plugin, version, digest)` keeps installation pinned to
  an exact version and artifact digest.
- Uninstall stays manual and confirmed. No layer here automates removal.
- `OperatorApproval` keeps rejecting automation-scoped device identities for
  Plugin writes. The layers above are not automation impersonating a person;
  they are distinct authority kinds that audit differently.

## Attribution

`Actor` in `src/plugin_operation.rs` is a closed enum serialized into
installation and uninstall intents and into the operation journal. Layers 2 and
3 each need a variant, or the audit chain breaks:

```rust
Actor::Policy   { source: String },   // "host:/var/lib/cowboy/deployment-policy.json"
Actor::Deployer { key_id: String },   // "ci-2026-09"
```

Removing human interaction must never remove the record of what acted.

### The attribution change is the expensive part

`Actor` is a closed tagged enum with `deny_unknown_fields`, and it is serialized
into `InstallIntent` and `UninstallIntent`, which persist in the durable
installation journal. A Controller that writes `{"kind":"policy", …}` therefore
produces records that **every older reader rejects outright** — the previous
Controller generation, the next-transaction recovery reader and every cold
reader. That is the `foreign-identity` case the installation reader floors exist
to cover.

So the policy engine is not the cost. The cost is a reader-first rollout:

1. Teach every reader to represent an unknown actor without failing, and accept
   that floor first. No writer may emit the new variant before this lands.
2. Only then add the writer and the policy engine.
3. Re-accept `just plugin-install-reader-conformance` (168 checks across
   immutable active, recovery and cold roles) and
   `just plugin-install-connected-conformance` (five real flows, nine reader
   pairs, 45 checks over 90 cold reads). Neither matrix exists in-tree; both
   must be authored against real Controller/Machine pairs.

Sequencing this wrong — shipping the writer before the readers — corrupts the
recovery path for real installations, which is why it is called out here rather
than left for whoever picks the layer up.

## Rollout

1. Layer 1, standalone and already shipped here.
2. Layer 3, because an explicitly triggered path is easier to verify than a
   policy-driven one. Contract and verification belong in
   `components/plugin-sdk`; `operator_approval.rs`, `plugin_install.rs` and its
   journal consume it.
3. Layer 2, built on Layer 3's mechanism with the trigger replaced by policy.
   Its `provider_idle` queueing wants the first two settled.

`docs/plugin-packages.md` and `.agents/skills/release-cowboy-plugin/` need the
matching change when Layer 2 or 3 lands: the skill's current "do not call a
Machine installation endpoint" step should become "do not substitute a person's
credential for the user's decision".
