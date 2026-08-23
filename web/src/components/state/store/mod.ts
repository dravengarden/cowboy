// Cowboy-owned state-store component: `persisted()` supplies per-device
// reactive state and `useStore(store)` binds the same contract to React.

export { persisted } from "./store.ts";
export type { KvStorage, PersistedOpts, ReadableStore, Store } from "./store.ts";
export { useStore } from "./use-store.ts";
