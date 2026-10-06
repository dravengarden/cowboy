# State leases — design and the reason not to build one yet

Status: design for review. Nothing is implemented, and the central finding is that
nothing should be until a real consumer exists. This addresses the "state leases and
policy" part of the P0 exit and the "exclusive fenced ownership" part of P3 in the
[completion ledger](plugin-refactor-completion.md). It follows the target text in
[the spatiotemporal design](plugin-spatiotemporal-design.md) (sections 4, 5.4, 6.1–6.3
and the stable-namespace rule: *instances hold authorized reader/writer leases over a
dataset bound to Service, security domain, dataset and logical scope; two incompatible
writers must never write one namespace; shared resources have a provider-held owner and
consumers hold bounded, owner-confirmed leases; disconnect is not release and expiry does
not prove an effect undone*).

## What exists, and what it already gives

- **Exclusivity by process.** Every durable namespace is owned by one process through an
  exclusive file lock held for that process's life, with retained directory and lock
  handles that end admission when replaced (`machine_broker/namespace.rs`, the Plugin
  operation journal, core security, telemetry file, logs storage, the local Operator
  endpoint). A second owner is refused at open.
- **Identity.** A Machine-minted Session lineage ([incarnation](plugin-session-incarnation-writer.md))
  names one life of a Session slot.
- **Process-local leases of other things.** Execution leases bind a Machine command to the
  original authenticated connection and receipt time; native buffer leases and read scopes
  bind Session/Workspace observations; release leases pin installed generations. None of
  them names a *dataset* or a reader/writer role.
- **Contract for datasets** ([state dataset design](plugin-state-dataset-design.md)): floors,
  declared schemas, exclusive ownership, recovery.

## The finding: there is no consumer

A lease is needed when more than one holder wants access to the same dataset at once, or
one holder wants to prove it still has the right to write. Today neither happens:

- Each durable dataset has exactly one owner process and exclusion is the OS lock. Readers
  are the owner itself, or a later process after the owner exited.
- No Plugin manifest in the repository declares a business dataset, state schema or
  reader/writer range (a search of the manifests finds only a GitHub API `state=` query
  parameter). The platform has no Plugin that owns data across generations.
- Old and new instances never write the same namespace concurrently; a Machine or Controller
  replacement stops the old owner first, or (for the planned rolling configuration check) the
  new instance only validates and writes nothing.

Building a lease type now would add a second concurrency mechanism beside the file lock with
nothing to exercise it, which is exactly the speculative generality the ledger warns against
("no second Plugin lifecycle, no unconstrained executor"). So this document fixes the
contract a future implementation must satisfy and the conditions that trigger it.

## Contract a lease must satisfy

A **state lease** is an in-memory handle issued by the dataset's owner. It is never
serialized as authority, and a copy in a log or record grants nothing.

1. **Owner-issued.** Only the party that holds the dataset issues leases (Machine for Machine
   datasets, Service for Service datasets). A consumer type has no constructor and no
   deserializer, as for the incarnation value.
2. **Key.** `(dataset identity, scope, role, holder)` where dataset identity is the contract's
   owner tuple (Machine, Service, security domain, dataset name); scope is the logical slot
   (for Session data `(slot, incarnation)`); role is Reader(schema interval) or Writer(schema);
   holder is the exact instance (release digest, installation generation, instance incarnation).
3. **Writer exclusion.** At most one Writer per `(dataset, scope)`. Readers may coexist only if
   their declared interval contains the writer's schema; otherwise issuing the writer is a
   maintenance action that is refused, never a preemption. The owner never lets a hot swap
   create two incompatible writers.
4. **Fence.** Every write presents the lease's monotonic fence and the owner rejects a lower
   one. Issuing a Writer advances the fence in the same commit as a durable per-dataset fence
   floor, written with the same staged/sync/rename/directory-sync commit as the datasets
   (`Namespace::commit`). Leases themselves are *not* durable: after an owner restart every
   lease is void and holders reacquire, but the fence floor survives so a restarted owner never
   issues a lower fence.
5. **Incarnation binding.** A lease for Session-scoped data names the Session incarnation. A
   reset or end voids it, and a late write from the old holder is rejected rather than written
   into the new lineage ("late results cannot write into another incarnation").
6. **States and honest outcomes.** Held, Suspended (the holder's authenticated connection is
   gone: writes refused, same holder identity may resume within a bound), Revoked, Expired,
   Released. Disconnect is not release. Expiry and revocation do not claim effects were undone:
   any in-flight write ends as Unknown and the scope enters NeedsReconcile, which a separate,
   independently authorized action resolves.
7. **Time.** Deadlines use the monotonic clock; a long-running holder renews on its own cadence,
   independent of any short RPC timeout, and a wall-clock rollback never extends a lease.
8. **GC.** A Reader lease pins its schema interval and a Writer pins the dataset, so retention
   and uninstall previews list them alongside Session/Workspace leases.
9. **Principal and policy.** Issue checks the current policy epoch and principal; a restored
   state that cannot prove the original authority still holds refuses new sensitive operations
   while the owner may still reclaim its own temporary resources.

## Evidence a future implementation owes

Acquire, renew and release; a second Writer refused and a Reader outside the interval refused;
fence monotonic across owner restart with SIGKILL at each commit stage (as for the deletion
journal and incarnation matrices); a stale holder rejected after an incarnation change; Suspended
on disconnect and a resume by the same holder only; expiry producing Unknown/NeedsReconcile and
never Success; clock-rollback refusal; old and new reader/writer releases of one dataset, using
exact immutable releases, against a real production holder across an activation. Fixtures never
stand in for that last item.

## When to build it

When one of these exists, not before:

- a Plugin (or core component) that **owns a durable dataset across generations**, with the old and
  new generation both able to hold it during a rollout; or
- a dataset that **two processes must hold at once**, for example Controller and Machine each writing
  slices of one Session's state; or
- an external effect that needs a **fencing token** so a stale holder cannot write after a handover.

At that point reuse `namespace.rs` for the fence floor, the incarnation for scope identity, and the
dataset contract's floors for compatibility rather than adding a parallel mechanism. Until then the
exit "state leases" stays open in the ledger, deliberately, with this document as its acceptance
contract rather than as a backlog item.
