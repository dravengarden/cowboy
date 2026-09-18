/** Local observation of existing owners, not a durable registry or an ID importer. */
import type { ReadableStore } from "@cowboy/state-store/core";
import type { BufferTarget, OwnedCodeBuffer, OwnerView } from "./owner.ts";
import { BufferClientError } from "./protocol.ts";
import { createSynchronizationProjection } from "./synchronizationProjection.ts";
import { createNavigationProjection } from "./navigationProjection.ts";

declare const cleanupHandle: unique symbol;
export interface CleanupHandle {
  readonly [cleanupHandle]: true;
}
export type CleanupStatus =
  | "working"
  | "context_lost"
  | "release_uncertain"
  | "unavailable"
  | "unknown"
  | "pending"
  | "synchronization"
  | "navigation"
  | "destination"
  | "needs_cleanup";
export interface CleanupRow {
  readonly handle: CleanupHandle;
  /** Page-local display identity, never reused or accepted from serialized data. */
  readonly ordinal: number;
  readonly target: BufferTarget | undefined;
  readonly status: CleanupStatus;
  readonly canInspect: boolean;
  readonly canContinue: boolean;
}
export interface CleanupView {
  readonly contextLost: boolean;
  readonly active: number;
  readonly rows: readonly CleanupRow[];
}
export interface CodeBufferCleanup extends ReadableStore<CleanupView> {
  inspect(handle: CleanupHandle, observer?: AbortSignal): Promise<void>;
  continueCleanup(handle: CleanupHandle): Promise<void>;
}

function status(view: OwnerView): CleanupStatus {
  if (view.contextLost) return "context_lost";
  if (view.busy || view.cleaning) return "working";
  if (view.synchronizing) return "synchronization";
  if (view.navigating) return "navigation";
  if (view.handingOff) return "destination";
  if (view.releaseAttempted) return "release_uncertain";
  if (view.failure || !view.fresh) return "unavailable";
  if (!view.observation || view.observation.state === "unknown") {
    return "unknown";
  }
  if (view.observation.pending) return "pending";
  return "needs_cleanup";
}

/** Owned by the same core registry. Constructing it acquires no subscription. */
export function createCleanupMonitor(context: AbortSignal) {
  const owners = new Map<OwnedCodeBuffer, {
    handle: CleanupHandle;
    ordinal: number;
    target: BufferTarget;
  }>();
  const handles = new WeakMap<CleanupHandle, OwnedCodeBuffer>();
  const listeners = new Set<{ listener: () => void }>();
  let ordinal = 0;
  let cached: CleanupView | undefined;
  let queued = false;
  let watching = false;
  let revision = 0;
  const changed = () => {
    ++revision;
    cached = undefined;
    if (queued || !listeners.size) return;
    queued = true;
    // UI callbacks must not re-enter a partially admitted owner operation.
    queueMicrotask(() => {
      queued = false;
      // Subscriptions created by a callback wait for the NEXT notification.
      // oxlint-disable-next-line unicorn/no-useless-spread
      for (const subscription of [...listeners]) {
        if (!listeners.has(subscription)) continue;
        try {
          subscription.listener();
        } catch {
          /* a broken observer cannot fail cleanup or other observers */
        }
      }
    });
  };
  const watch = () => {
    const needed = (owners.size > 0 || listeners.size > 0) && !context.aborted;
    if (needed === watching) return;
    watching = needed;
    if (needed) context.addEventListener("abort", ended, { once: true });
    else context.removeEventListener("abort", ended);
  };
  const ended = () => {
    watching = false;
    changed();
  };
  const eligible = (handle: CleanupHandle): OwnedCodeBuffer => {
    if (context.aborted) throw new BufferClientError("context_lost");
    const owner = handles.get(handle);
    if (!owner || !owners.has(owner)) throw new BufferClientError("state");
    const view = owner.view();
    if (!view.closing || !view.resourceId) throw new BufferClientError("state");
    if (view.busy || view.cleaning) throw new BufferClientError("busy");
    if (view.synchronizing || view.navigating) {
      throw new BufferClientError("state");
    }
    return owner;
  };
  const store: CodeBufferCleanup = Object.freeze({
    get() {
      if (cached?.contextLost === context.aborted) return cached;
      let active = 0;
      const rows: CleanupRow[] = [];
      for (const [owner, entry] of owners) {
        const view = owner.view();
        if (!view.closing && !view.contextLost) {
          active++;
          continue; // never offer cleanup of a still-active consumer
        }
        const canInspect = !view.contextLost && view.closing &&
          !!view.resourceId && !view.busy && !view.cleaning;
        rows.push(Object.freeze({
          handle: entry.handle,
          ordinal: entry.ordinal,
          target: view.contextLost ? undefined : entry.target,
          status: status(view),
          canInspect: canInspect && !view.synchronizing && !view.navigating,
          canContinue: canInspect && !view.synchronizing && !view.navigating &&
            !view.releaseAttempted,
        }));
      }
      cached = Object.freeze({
        contextLost: context.aborted,
        active,
        rows: Object.freeze(rows),
      });
      return cached;
    },
    subscribe(listener: () => void) {
      const subscription = { listener };
      listeners.add(subscription);
      watch();
      return () => {
        listeners.delete(subscription);
        watch();
      };
    },
    async inspect(handle: CleanupHandle, observer?: AbortSignal) {
      await eligible(handle).observe(observer);
    },
    async continueCleanup(handle: CleanupHandle) {
      const owner = eligible(handle);
      if (owner.view().releaseAttempted) throw new BufferClientError("state");
      // close() claims synchronously. A second panel/click cannot start a pass.
      await owner.close();
    },
  });
  return {
    store,
    synchronizations: createSynchronizationProjection(context, {
      subscribe: store.subscribe,
      revision: () => revision,
      entries: () => owners,
    }),
    navigations: createNavigationProjection(context, {
      subscribe: store.subscribe,
      revision: () => revision,
      entries: () => owners,
    }),
    changed,
    size: () => owners.size,
    retained: () => Object.freeze([...owners.keys()]),
    add(owner: OwnedCodeBuffer, target: BufferTarget) {
      if (ordinal >= Number.MAX_SAFE_INTEGER) {
        throw new BufferClientError("capacity");
      }
      const handle = Object.freeze({}) as CleanupHandle;
      handles.set(handle, owner);
      owners.set(owner, { handle, ordinal: ++ordinal, target });
      watch();
      changed();
    },
    retire(owner: OwnedCodeBuffer) {
      owners.delete(owner);
      watch();
      changed();
    },
  };
}
