// Cowboy-owned state-store component: `persisted()` supplies per-device
// reactive state and `useStore(store)` binds the same contract to React.

export { persisted } from "./store.ts";
export type {
  DisposableStore,
  KvStorage,
  PersistedOpts,
  PersistenceError,
  ReadableStore,
  StorageChange,
  StorageChanges,
  StorageCodec,
  Store,
} from "./store.ts";
export { useStore } from "./use-store.ts";
