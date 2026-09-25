# Service-side Machine Plugin membership

Which Plugins a Machine runs is a Service-side declaration, not something
somebody presses in a client. A declared Machine's clients offer no install,
upgrade or uninstall action at all; convergence applies the declaration.

## The document

`<data-dir>/machine-plugins.json`, re-read in place by the Controller and by
every host-authorized convergence run:

```json
{
  "schema": 1,
  "machines": {
    "hawk":   { "plugins": ["claude-code", "codex", "zed"], "unlisted": "keep" },
    "falcon": { "plugins": ["claude-code", "codex"], "unlisted": "uninstall" }
  }
}
```

A Machine that the document does not name is **unmanaged**: nothing is added or
removed there and its clients keep the ordinary lifecycle actions. Declaring a
Machine is what moves the decision Service-side, so adoption is per Machine and
reversible by deleting its entry.

`unlisted` decides what happens to an installed Plugin the list does not name.
It defaults to `keep`, so adding a Machine to the document can never remove
anything by omission alone; `uninstall` is the explicit opt-in to removal.

An absent document means nothing is declared. An unreadable or invalid one
keeps the last accepted declaration and is logged: a half-written file must
never be read as "uninstall everything". A Plugin id that is not a plain
lowercase slug is rejected rather than treated as "this Machine should not have
it".

## What convergence does with it

`cowboy operator converge` plans, per declared Machine:

- **install** every declared Plugin the Machine lacks, at the newest `ready`
  Catalog release that declares the Machine's exact platform;
- **upgrade** every declared Plugin that is behind that release;
- **remove** every installed Plugin the list does not name, when `unlisted` is
  `uninstall`.

Removal runs the same durable transaction a browser confirmation uses: it takes
the uninstall preview first, and **refuses as soon as the preview names any
affected session**. Automation never sends the active-session confirmation. A
live conversation is not ended to satisfy a list; the Plugin stays installed
until it is idle, and the next run removes it.

Everything else convergence already refuses still applies: a release that is
not `ready` or that does not declare the platform is not a target, a Plugin
holding an active session lease is not recycled under a live worker, an
installed version ahead of the Catalog is never downgraded, and each step
carries one deterministic operation identity.

## Why the client has no buttons

A declared Machine's membership has exactly one source. A button that could
install or remove a Plugin there could only disagree with the declaration and
with the convergence that applies it, so the host blocks those capabilities and
the Provider's own settings surface renders nothing for them.

That is presentation; the authority is the Controller, which refuses
client-initiated installs, uninstall previews and uninstalls for a declared
Machine with HTTP 409. An older client, a direct API call or a stale tab cannot
fork the membership. Convergence itself does not pass through that refusal: it
runs under the host grant on the private Operator endpoint, which is the
authorized caller the installation transaction has always required.

## Operating it

- Add a Plugin to a Machine: name it in that Machine's list. The next
  `cowboy operator converge --apply` run installs it.
- Remove one: take it out of the list on a Machine whose policy is `uninstall`,
  and make sure its sessions are finished.
- Take a Machine back under manual control: delete its entry. Its Plugins stay
  exactly as they are and its clients offer the ordinary actions again.
- Stop everything: `cowboy operator freeze`. See
  [Machine component convergence](machine-component-convergence.md#stopping-it).
- If a dispatched installation lost its response, inspect its operation and run
  `cowboy operator reconcile-install` with that exact operation ID. This queries
  the Machine's durable receipt; it does not repeat the installation. A failed
  pre-activation Staging receipt is released only after the same action also
  verifies the Machine still has the original durable target. Retry that release
  with a fresh operation ID after the resolution succeeds.
