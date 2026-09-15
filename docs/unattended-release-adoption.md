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

Move the authority for unattended delivery from *who is logged in* to *what is
signed* and *what policy declares*. Every layer below keeps a verifiable
authority and a recorded actor; none of them removes one.

An unauthenticated installation endpoint is explicitly rejected as a design: it
would let anything that can reach the Controller place arbitrary executables on
every enrolled Machine. The goal is no *human interaction*, not no *authority*.

## Layer 1 — Catalog watch (implemented)

`src/plugin_catalog/watch.rs` watches each Catalog root and re-runs the same
`refresh_with_runtime` + `ProviderCatalog::refresh_external` pair that startup
and the admin endpoint already run. It introduces no endpoint, no credential and
no new trust boundary: writing to the Catalog directory was already the
publication boundary.

The watch is safe because publication is already crash-safe for a concurrent
reader. `copyImmutable` stages into a temporary file and hard-links it into
place, and the release envelope is linked **last** as the commit marker. A
reload therefore observes either the previous Catalog or one complete immutable
release, never a half-transaction.

- A publication burst — artifacts, package, host bundle, envelope — is collapsed
  by a settle window into one reload.
- A failed reload leaves the previous snapshot visible, exactly as the admin
  endpoint does, and retries on a bounded backoff so a release is not stranded
  until some unrelated directory write.
- An absent root is not an error; the legacy compatibility directory is often
  missing.
- `POST /api/plugins/catalog/refresh` remains as an idempotent manual fallback.

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
installation and uninstall intents and into the operation journal. Layers 2 and 3
each need a variant, or the audit chain breaks:

```rust
Actor::Policy   { source: String },   // "host:/var/lib/cowboy/deployment-policy.json"
Actor::Deployer { key_id: String },   // "ci-2026-09"
```

Removing human interaction must never remove the record of what acted.

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
