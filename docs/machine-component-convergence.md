# Machine component convergence

The Controller owns one signed desired component set and converges every
connected Machine to it continuously. Nobody presses Update for an automatic
component; the Machine still verifies everything before it activates one.

## What authorizes an update

`COWBOY_MACHINE_COMPONENTS_MANIFEST` is the only authority. Each record names
an exact version, artifact URL, SHA-256 digest, publisher signature, and — for
an automatic component — a readiness probe. The browser, the Web API and this
convergence loop can all ask for a reconcile; none of them can supply an
artifact URL, digest or signature. A Machine independently re-downloads,
re-hashes and re-verifies the signature, runs the probe against the staged
generation, and keeps the previous generation for rollback.

`automatic: true` is the per-component decision to converge without a person.
The Controller refuses a manifest whose automatic record has no probe, and so
does the Machine.

## When it runs

The Controller re-reads the manifest and compares every connected Machine with
it once a minute, and each Machine additionally reconciles its automatic set at
welcome. A published component therefore reaches a Machine that is already
connected, without a Controller restart and without a reconnect.

A Machine that connected less than two minutes ago is skipped: it is already
reconciling the set its welcome carried, and a second dispatch would activate
the same generation twice.

A rejected manifest — unreadable, unparsable, unsigned, plaintext-HTTP,
duplicated, digest-shaped wrongly, or automatic without a probe — leaves the
last accepted set in force and is logged. A half-read or invalid file never
becomes a desired state, and it never empties one.

## What it refuses to do

- **Never a leased generation.** A component whose installed generation still
  has `active_leases > 0` is reported as `draining` and left alone. Replacing
  runtime bytes under a live session is an explicit maintenance decision, so it
  stays with the per-component action in Machines settings.
- **Never a manual component.** Records without `automatic` converge only when
  somebody asks for them.
- **Never twice at once.** One Reconcile per Machine is in flight at a time.
- **Never forever.** Every dispatch is counted before it is sent, so the
  backoff (1, 2, 4, 8 minutes, capped at 30) covers a Machine that acknowledges
  each Reconcile without ever activating the component as well as one that
  fails outright. After four attempts that component is `blocked` with the
  reason until a new digest is published. Reconnecting does not clear it; a
  published fix does, because the block is bound to the exact digest that
  failed. Only an inventory that shows the digest active retires an attempt.

## What the fleet reports

`MachineSummary.convergence` carries one entry per automatic component that has
not converged:

| state | meaning |
|---|---|
| `pending` | eligible; dispatched on this pass |
| `draining` | a live session still leases the installed generation |
| `verifying` | the Reconcile was acknowledged; waiting for the inventory that proves the digest is active |
| `retrying` | the last dispatch failed; waiting out its backoff |
| `blocked` | repeated failure against these exact bytes; needs a person |

A converged component has no entry. The list is a projection — it never
authorizes an install by itself.

## Operating it

- Publish a new component: sign it, add or update its record in the manifest,
  and write the file. Convergence picks it up within a minute.
- Hold a component back: publish it with `automatic: false`; it then shows as
  an available update and waits for the per-component action.
- Stop a bad rollout: restore the previous record in the manifest. The next
  reload treats it as the desired state and converges the fleet back to it.
- A `blocked` component means four counted attempts against those exact bytes.
  Read the Machine's journal before publishing anything else; a new operation
  identity is not a recovery mechanism.
