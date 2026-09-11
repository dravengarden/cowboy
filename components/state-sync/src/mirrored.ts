// Mirrored store — the STATE-BASED (CvRDT) tier of the authority spectrum: a
// local-first reactive value mirrored to a PASSIVE remote (a dumb key-value the
// server just stores + returns). No mutation log, no arbiter — conflict is
// resolved by a `reconcile(local, remote)` MERGE over whole values
// (last-writer-wins by default).
//
// This is the right tier when there is no concurrent multi-writer convergence to
// guarantee — per-account settings / progress that one device edits at a time
// (liveview: audio position/rate/sleep, reading progress, book prefs). For
// concurrent multi-writer state that must converge operationally, use
// `replicatedStore` (the OP-based tier) — the read side (`get`/`subscribe`) is
// identical, so the same `useStore` renders either and a state can be promoted
// from one tier to the other without touching the components.

import type { Store } from "@cowboy/state-store";
import {
  createOwnedResourceScope,
  type ScopeSnapshot,
} from "@cowboy/state-store/scope";
import type { LocalPersistence, RemoteBackend } from "./types.ts";

export type SyncStatus = "connecting" | "live" | "offline";

/** A `Store<T>` whose value is mirrored to a passive remote. Write with `set`
 *  (the state-based dual of `replicatedStore.mutate`). `hydrate` then `connect`
 *  on startup; `flush` on `pagehide`. */
export interface MirroredStore<T> extends Store<T> {
  /** Load the local mirror (this device's last value) for an instant first paint.
   *  Call once BEFORE `connect`. No-op without a `local` backend or stored value. */
  hydrate(): Promise<void>;
  /** Pull the remote, `reconcile` it with the current value, and (if the backend
   *  supports it) subscribe to live remote changes. Idempotent. */
  connect(): void;
  /** Force any debounced local + remote writes out NOW (call on `pagehide`). */
  flush(): Promise<void>;
  /** Retire the current observation (including late load/subscribe callbacks).
   * The writable store remains usable; use dispose to end its lifetime. */
  disconnect(): void;
  readonly status: SyncStatus;
  /** Seal this instance; cancel unsubmitted remote timers, drain admitted work,
   * and persist the local mirror. Does not delete data or reverse remote saves.
   * A failed final save/cleanup rejects and stays visible in lifecycle. */
  dispose(): Promise<void>;
  readonly lifecycle: ScopeSnapshot;
}

export interface MirroredOpts<T> {
  initial: T;
  remote: RemoteBackend<T>;
  /** Merge the remote value into local on connect / live update. Default:
   *  remote-wins (`(_local, remote) => remote`). Provide a domain merge for
   *  smarter resolution — e.g. liveview's "adopt the server's audio position only
   *  if it is >8s ahead on the same chapter". MUST be pure. */
  reconcile?: (local: T, remote: T) => T;
  /** Instant offline mirror of the whole value (localStorage/IDB adapter). */
  local?: LocalPersistence<T>;
  /** Remote-write pacing. `debounceMs`: save `ms` after writes settle (reading
   *  progress). `throttleMs`: save at most once per `ms`, trailing (audio
   *  position). Neither: save on every `set`. */
  push?: { debounceMs?: number; throttleMs?: number };
  /** Debounce (ms) for the local mirror save. Default 250. */
  localDebounceMs?: number;
  /** Surface a remote load/save rejection (default: swallow — the local mirror
   *  keeps the app fully usable offline). */
  onError?: (error: unknown) => void;
}

export function mirroredStore<T>(opts: MirroredOpts<T>): MirroredStore<T> {
  const { initial, remote, local } = opts;
  const scope = createOwnedResourceScope();
  const reconcile = opts.reconcile ?? ((_local: T, r: T): T => r);
  const listeners = new Set<{ listener: () => void }>();
  let value = initial;
  let revision = 0;
  let status: SyncStatus = "connecting";
  let hydration: Promise<void> | undefined;
  type Connection = { release: () => Promise<void> };
  let connection: Connection | undefined;
  let localTimer: ReturnType<typeof setTimeout> | undefined;
  let remoteTimer: ReturnType<typeof setTimeout> | undefined;
  let localDirty = false;
  let remoteDirty = false;
  let localTail = Promise.resolve();
  let remoteTail = Promise.resolve();

  const report = (error: unknown): void => {
    if (!scope.active) return;
    try {
      opts.onError?.(error);
    } catch { /* diagnostics are not authority */ }
  };
  const emit = (): void => {
    for (const subscription of [...listeners]) {
      if (!scope.active || !listeners.has(subscription)) continue;
      try {
        subscription.listener();
      } catch {
        console.warn("mirror subscriber failed");
      }
    }
  };
  const setValue = (next: T): void => {
    if (!scope.active || Object.is(next, value)) return;
    value = next;
    revision++;
    emit();
  };
  const clearTimers = (): void => {
    if (localTimer !== undefined) clearTimeout(localTimer);
    if (remoteTimer !== undefined) clearTimeout(remoteTimer);
    localTimer = remoteTimer = undefined;
  };
  // Serialize each backend independently. A slow older save must never finish
  // after a newer write and become the lasting value.
  const saveLocal = (): Promise<void> => {
    if (!local) return Promise.resolve();
    const next = value;
    localDirty = false;
    const write = localTail.then(() => local.save(next));
    localTail = write.catch((error: unknown) => {
      localDirty = true;
      report(error);
    });
    return write;
  };
  const saveRemote = (): Promise<void> => {
    const next = value;
    remoteDirty = false;
    const write = remoteTail.then(() => remote.save(next));
    remoteTail = write.catch(report);
    return write;
  };
  const scheduleLocal = (): void => {
    if (!local || !scope.active) return;
    localDirty = true;
    if (localTimer !== undefined) clearTimeout(localTimer);
    localTimer = setTimeout(() => {
      localTimer = undefined;
      if (scope.active) void saveLocal().catch(() => undefined);
    }, opts.localDebounceMs ?? 250);
  };
  const scheduleRemote = (): void => {
    if (!scope.active) return;
    remoteDirty = true;
    const debounce = opts.push?.debounceMs;
    const throttle = opts.push?.throttleMs;
    if (debounce === undefined && throttle === undefined) {
      void saveRemote().catch(() => undefined);
      return;
    }
    if (debounce !== undefined && remoteTimer !== undefined) {
      clearTimeout(remoteTimer);
    }
    if (debounce !== undefined || remoteTimer === undefined) {
      remoteTimer = setTimeout(() => {
        remoteTimer = undefined;
        if (scope.active && remoteDirty) {
          void saveRemote().catch(() => undefined);
        }
      }, debounce ?? throttle);
    }
  };
  const disconnect = (): void => {
    const retired = connection;
    if (!retired && status === "offline") return;
    connection = undefined; // revoke BEFORE invoking external unsubscribe
    status = "offline";
    if (retired) void retired.release().catch(() => undefined); // scope retains failures
    emit();
  };

  // The store owns its timers, local mirror and borrowed remote observation.
  // The backend itself may be shared; never dispose it, delete its data, or
  // manufacture a compensating remote write here.
  scope.defer(async () => {
    clearTimers();
    await Promise.all([localTail, remoteTail]);
    if (localDirty) await saveLocal(); // strict final local durability barrier
  });

  return {
    get: (): T => value,
    subscribe: (listener): () => void => {
      scope.assertActive();
      const subscription = { listener };
      listeners.add(subscription);
      return () => {
        listeners.delete(subscription);
      };
    },
    set: (next): void => {
      scope.assertActive();
      const resolved = typeof next === "function"
        ? (next as (prev: T) => T)(value)
        : next;
      scope.assertActive();
      if (Object.is(resolved, value)) return;
      value = resolved;
      revision++;
      // Register the pending local value before an observer can dispose us.
      scheduleLocal();
      scheduleRemote();
      emit();
    },
    hydrate: (): Promise<void> => {
      scope.assertActive();
      if (hydration) return hydration;
      if (!local) return Promise.resolve();
      const started = revision;
      let begin!: (task: Promise<void>) => void;
      // Reserve identity before a synchronous/reentrant backend starts.
      // oxlint-disable-next-line promise/avoid-new
      hydration = new Promise<void>((resolve) => {
        begin = resolve;
      });
      begin(scope.run(async () => {
        if (!scope.active) return;
        try {
          const cached = await local.load();
          if (scope.active && revision === started && cached !== null) {
            setValue(cached);
          }
        } catch (error) {
          report(error);
        }
      }));
      return hydration;
    },
    connect: (): void => {
      scope.assertActive();
      if (connection) return;
      let stop: (() => void) | undefined;
      const release = scope.defer(() => {
        const cleanup = stop;
        stop = undefined;
        cleanup?.();
      });
      const current: Connection = { release };
      connection = current;
      const isCurrent = (): boolean => scope.active && connection === current;
      revision++; // a remote observation outranks an earlier local cache read
      status = "connecting";
      emit();
      if (!isCurrent()) return;
      void scope.run(async () => {
        try {
          const incoming = await remote.load();
          if (!isCurrent()) return;
          if (incoming !== null) {
            const next = reconcile(value, incoming);
            if (!isCurrent()) return;
            setValue(next);
          }
          if (!isCurrent()) return;
          stop = remote.subscribe?.((incoming) => {
            if (!isCurrent()) return;
            const next = reconcile(value, incoming);
            if (isCurrent()) setValue(next);
          });
          // subscribe may synchronously call back and disconnect/reconnect.
          // Its pre-registered holder still releases the returned old handle.
          if (!isCurrent()) return;
          status = "live";
          emit();
        } catch (error) {
          if (!isCurrent()) return;
          disconnect();
          report(error);
        }
      }).catch(report);
    },
    disconnect,
    flush: (): Promise<void> => {
      return scope.run(async () => {
        clearTimers();
        const writes = [
          remoteDirty ? saveRemote() : remoteTail,
          localDirty ? saveLocal() : localTail,
        ];
        // Explicit barriers surface failures. Background writes remain usable
        // offline and report through onError.
        await Promise.all(writes);
      });
    },
    dispose: (): Promise<void> => {
      const done = scope.dispose(); // synchronous callback fence
      listeners.clear();
      clearTimers();
      remoteDirty = false; // no new remote effect in a cleanup
      disconnect();
      return done;
    },
    get status(): SyncStatus {
      return status;
    },
    get lifecycle(): ScopeSnapshot {
      return scope.snapshot();
    },
  };
}
