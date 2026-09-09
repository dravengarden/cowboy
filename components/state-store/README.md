# State store

`@cowboy/state-store/core` is framework-neutral. The root export additionally
provides `useStore` for React; the core entry does not import React. React is an
optional peer: core-only consumers do not need to install it, while users of the
root React adapter must supply React 19.

```ts
import { persisted } from "@cowboy/state-store/core";

const expanded = persisted("expanded", false, {
  serialize: (value) => value ? "1" : "0",
  deserialize: (raw) => {
    if (raw !== "1" && raw !== "0") throw new Error("invalid boolean");
    return raw === "1";
  },
});
const unsubscribe = expanded.subscribe(() => render(expanded.get()));
unsubscribe(); // releases this subscription, not the shared store
expanded.dispose(); // only the instance's owner may revoke the whole store
```

Both codec functions are required in v2. A decoder validates untrusted bytes and
returns a normalized value or throws; there is no implicit JSON type assertion.
Existing Cowboy preference codecs retain their keys and wire encodings.

Construction performs an initial read but installs no global listener. The first
subscription acquires the storage listener and the last unsubscribe releases it.
Re-subscription and detached `get()` reconcile external changes; unchanged raw
bytes preserve object snapshot identity. Watched reads use the cached snapshot.
Only matching storage-area events affect the store, including `clear()` events.

Each instance and each listener lifetime is independently fenced. `dispose()` is
idempotent, stops listening, clears subscribers, and rejects later `set()` or
`subscribe()`. `get()` then returns the final snapshot. Cleanup never removes a
preference, cancels a remote operation, or closes a Machine session. Core-root
singleton preferences share subscriptions; a React view must not dispose them.
View-owned instances should be disposed by that view's lifecycle owner.

`storage: null` explicitly selects memory only. `storage`, `changes` and
`onError` are injectable. Read failures retain the last snapshot; malformed
stored bytes fall back to the initial value without rewriting storage. Encoding
and write failures still commit and notify in memory. The same old backend bytes
do not undo a failed write; a subsequent external change can replace it.
`onError` reports only a phase, never a key, payload or exception message. It is
best-effort and cannot break the state update. Throwing subscribers do not
prevent other subscribers from observing a committed update; their failures are
rethrown together. Custom event sources must acquire atomically and return a
cleanup function; callbacks queued before cleanup are ignored afterwards.

This is small synchronous preference state, not a durable effect journal,
cross-process transaction, or recovery mechanism.
