/** Passive, page-local recovery projection; never executes or imports a group. */
import type { ReadableStore } from "@cowboy/state-store/core";
import type { BufferTarget, OwnedCodeBuffer } from "./owner.ts";
import type { NavigationView, OwnedNavigation } from "./navigation.ts";
import { BufferClientError } from "./protocol.ts";

declare const navigationHandle: unique symbol;
export interface NavigationHandle {
  readonly [navigationHandle]: true;
}
export type NavigationStatus =
  | "working"
  | "context_lost"
  | "release_uncertain"
  | "unavailable"
  | "unknown"
  | "pending"
  | "prepared"
  | "retained";
export interface NavigationRow {
  readonly handle: NavigationHandle;
  readonly ordinal: number;
  readonly target: BufferTarget | undefined;
  readonly status: NavigationStatus;
  readonly canInspect: boolean;
  readonly canRelease: boolean;
}
export interface NavigationsView {
  readonly contextLost: boolean;
  readonly rows: readonly NavigationRow[];
}
export interface CodeBufferNavigations extends ReadableStore<NavigationsView> {
  inspect(handle: NavigationHandle, observer?: AbortSignal): Promise<void>;
  release(handle: NavigationHandle, observer?: AbortSignal): Promise<void>;
}

function status(view: NavigationView): NavigationStatus {
  if (view.contextLost) return "context_lost";
  if (view.busy) return "working";
  if (view.releaseAttempted) return "release_uncertain";
  if (!view.fresh) {
    return view.executeAttempted && view.observation.state !== "retained"
      ? "unknown"
      : "unavailable";
  }
  if (view.observation.pending) return "pending";
  if (view.observation.state === "retained") return "retained";
  if (view.observation.state === "prepared" && !view.executeAttempted) {
    return "prepared";
  }
  return "unknown";
}

export function createNavigationProjection(context: AbortSignal, source: {
  subscribe(listener: () => void): () => void;
  revision(): number;
  entries(): ReadonlyMap<OwnedCodeBuffer, { target: BufferTarget }>;
}): CodeBufferNavigations {
  let ordinal = 0;
  let revision = -1;
  let cached: NavigationsView | undefined;
  const handles = new WeakMap<
    OwnedNavigation,
    { handle: NavigationHandle; ordinal: number }
  >();
  const owners = new WeakMap<
    NavigationHandle,
    { owner: OwnedCodeBuffer; operation: OwnedNavigation }
  >();
  const original = (handle: NavigationHandle) => {
    if (context.aborted) throw new BufferClientError("context_lost");
    const entry = owners.get(handle);
    if (
      !entry || !source.entries().has(entry.owner) ||
      entry.owner.navigation() !== entry.operation
    ) {
      throw new BufferClientError("state");
    }
    return entry.operation;
  };
  return Object.freeze({
    subscribe: source.subscribe,
    get() {
      if (
        cached && revision === source.revision() &&
        cached.contextLost === context.aborted
      ) return cached;
      const rows: NavigationRow[] = [];
      for (const [owner, entry] of source.entries()) {
        const operation = owner.navigation();
        if (!operation) continue;
        let identity = handles.get(operation);
        if (!identity) {
          if (ordinal >= Number.MAX_SAFE_INTEGER) {
            throw new BufferClientError("capacity");
          }
          identity = {
            handle: Object.freeze({}) as NavigationHandle,
            ordinal: ++ordinal,
          };
          handles.set(operation, identity);
          owners.set(identity.handle, { owner, operation });
        }
        const view = operation.view();
        rows.push(Object.freeze({
          ...identity,
          target: context.aborted ? undefined : entry.target,
          status: status(view),
          canInspect: view.canInspect,
          canRelease: view.canRelease,
        }));
      }
      revision = source.revision();
      cached = Object.freeze({
        contextLost: context.aborted,
        rows: Object.freeze(rows),
      });
      return cached;
    },
    async inspect(handle: NavigationHandle, observer?: AbortSignal) {
      await original(handle).observe(observer);
    },
    async release(handle: NavigationHandle, observer?: AbortSignal) {
      await original(handle).release(observer);
    },
  });
}
