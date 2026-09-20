# Machine-owned Workspace root identity

The Controller already retains a continuous
[Workspace read observation](plugin-workspace-read-scopes.md) and a separate
[Session read route](plugin-session-read-routes.md). Both end only on events the
Controller itself accepts: inventory change, duplicate/invalid roots, budget
refusal, disconnect and same-epoch connection replacement. Neither notices that
the advertised root directory on the Machine has been replaced by a different
filesystem object while the Machine stayed connected and kept advertising the
same id and the same absolute path.

This slice gives that identity an owner. It is not a general graph executor,
a state reader/writer lease, a Plugin capability or a recovery mechanism.

## The hole this closes

An advertised root can be deleted and recreated, swapped for another worktree,
or re-bound to a different mount at the same absolute path. Today the Controller
preserves the exact same `WorkspaceCodeScope` across that replacement, because
id, canonical path and connection are all unchanged. Everything keyed by that
scope survives with it: buffered page/diff continuations, cached representation
ETags and conditional `304` replies, and remote reads already admitted against
the old observation. A consumer therefore continues one logical read sequence
across two unrelated filesystem objects, and a `304` can assert that content it
never observed is still current.

The Controller cannot close this by itself for a remote Machine. It has no
inode, no mount table and no configured root on its own host; it can only
compare strings the Machine sent it. Canonicalising a remote path on the
Controller would be a fabricated proof, and device/inode numbers are reused.

## Ownership

- **Machine** owns the identity. It resolves each advertised root, observes the
  object behind it, and mints an opaque `incarnation` the first time it sees a
  given object for that path. A fresh observation of a different object mints a
  fresh incarnation; nothing renews an old one. The registry is process-local,
  bounded, never persisted and never rebuilt from a Controller value.

  Device and inode numbers alone would be unsound here, because the kernel
  reuses them: a directory deleted and recreated at the same path frequently
  lands on the same inode, and a source test demonstrates exactly that. The
  registry therefore keeps one open directory handle per tracked root for as
  long as its identity exists. That handle reserves the original inode, so a
  replacement is necessarily a different object. Creation time is compared as
  an additional axis where the filesystem records it. The handle is never read
  or written and grants the Machine no access it did not already have; it is an
  open file-descriptor budget, so a configuration beyond 256 advertised roots
  publishes no identity for the excess, which the Controller then refuses
  rather than reads unfenced.
- **Machine** also enforces it. Every workspace-scoped adapter request carries
  the exact incarnation the Controller observed. The Machine re-resolves the
  root and refuses before any read when the live object is not the one that
  minted the carried incarnation. This check is per request, so it does not
  depend on an inventory refresh having happened.
- **Controller** only carries. It stores the opaque value inside the private
  `WorkspaceCodeScope` identity, requires equality to preserve a scope across
  an inventory, and hands it back verbatim on dispatch. It has no constructor,
  no serde path and no comparison by any derived value.

  It also acts on the Machine's *typed* refusal: a closed
  `WorkspaceRootIdentityChanged` value — not a free-text detail string — retires
  exactly that observation. Without this the Controller's own caches, ETags and
  page/diff continuations would keep answering for the replaced root until the
  next inventory arrived, and the Machine-side fence would only cover fresh
  dispatches. Retirement ends one slot: the connection, the Machine and every
  other advertised root keep their own observations, and the next inventory
  mints a new identity so a fence is not a permanent loss of the root.
- **core / Web / Plugin / native** own nothing here. The incarnation never
  enters the client-facing machine projection, the durable `inventory`
  document, a Plugin API, a URL or a cache key that leaves the Controller.

## Authority, identity, deadline, evidence

These stay separate, as in the preceding finite slices:

- *Authority* remains the original product credential and role observation from
  [product permission lifetimes](plugin-product-permission-lifetimes.md) and
  [buffered read authority](plugin-code-read-authority.md). A root incarnation
  authorises nothing; it can only end a read.
- *Identity* is the Machine-owned root observation added here, checked on top
  of the existing Service/Machine Site, connection epoch and logical scope.
- *Deadline* is unchanged. The original absolute request budget from
  [continuation finalization](plugin-continuation-finalization.md) still spans
  admission, the owned task and the response. A refusal does not extend it.
- *Effect evidence* is untouched. These are reads. Owned native buffer,
  synchronization and navigation resources keep their own lifetimes; a refused
  read neither releases, retires nor restores any of them.
- *Recovery lease* — none. Nothing here is durable and nothing is restored.

## Reversibility

A refused request performs no filesystem read, so there is nothing to reverse.
Requests already dispatched to the native/Code adapter before a replacement are
**not** cancelled: the Machine refuses new work, and a reply that arrives for an
ended observation is discarded rather than delivered. That is refusal, never
proof that the remote read stopped. An ended scope is recorded as ended; it is
never presented as a rollback, an undo or a restored continuation.

## Out of scope

Explicitly not closed by this slice, and still open in the
[completion ledger](plugin-refactor-completion.md):

- Session read routes whose cwd is a session worktree rather than an advertised
  root. Those keep only the existing Controller observation fence.
- Controller-local (colocated) execution. The Controller executes those reads
  itself and still has no continuous root fence of its own.
- General graph contracts, state-dataset identity and reader/writer leases,
  security-domain epochs, independently authorized post-effect restoration,
  native-generation acceptance and supported-device acceptance.
- Machine activation. This changes the Machine wire protocol to 22; the
  resident Machine keeps protocol 21 until a separate maintenance boundary.
  Against a protocol-21 Machine the Controller behaves exactly as before.

## Acceptance

The [accepted Controller rollout](releases/plugin-machine-workspace-identity-2026-09-20.md)
records the source negatives verified against the previous implementation, the
retained-handle proof, the complete quality gate, two v12/32-check connected
chains plus one against the published artifact, the production Controller's
recorded HTTP 200 for a replaced root, 24 browser cases and actual
Catalog/host floors before and after activation. Only the Controller is
activated; the resident protocol-21 Machine is unchanged.
