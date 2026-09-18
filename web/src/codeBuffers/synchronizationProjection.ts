/** Bounded local UI projection. No durable import, target lookup or polling. */
import type { ReadableStore } from "@cowboy/state-store/core";
import type { BufferTarget, OwnedCodeBuffer } from "./owner.ts";
import type { ContentIdentity } from "./content.ts";
import { BufferClientError } from "./protocol.ts";
import type {
  OwnedSynchronization,
  SynchronizationAction,
  SynchronizationConfirmation,
  SynchronizationView,
} from "./synchronization.ts";

declare const syncHandle: unique symbol;
export interface SynchronizationHandle {
  readonly [syncHandle]: true;
}
export type SynchronizationStatus =
  | "working"
  | "context_lost"
  | "retirement_uncertain"
  | "unavailable"
  | "unknown"
  | "pending"
  | "prepared"
  | "applied"
  | "changed"
  | "source"
  | "shared"
  | "budget";
export interface SynchronizationRow {
  readonly handle: SynchronizationHandle;
  readonly ordinal: number;
  readonly target: BufferTarget | undefined;
  readonly content: ContentIdentity | undefined;
  readonly status: SynchronizationStatus;
  readonly canApply: boolean;
  readonly canRetire: boolean;
  readonly canInspect: boolean;
}
export interface SynchronizationsView {
  readonly contextLost: boolean;
  readonly rows: readonly SynchronizationRow[];
}
export interface CodeBufferSynchronizations
  extends ReadableStore<SynchronizationsView> {
  inspect(handle: SynchronizationHandle, observer?: AbortSignal): Promise<void>;
  preview(
    handle: SynchronizationHandle,
    action: SynchronizationAction,
  ): SynchronizationConfirmation;
  isCurrent(
    handle: SynchronizationHandle,
    token: SynchronizationConfirmation,
  ): boolean;
  confirm(
    handle: SynchronizationHandle,
    token: SynchronizationConfirmation,
    observer?: AbortSignal,
  ): Promise<void>;
}

function status(view: SynchronizationView): SynchronizationStatus {
  if (view.contextLost) return "context_lost";
  if (view.busy) return "working";
  if (view.retireAttempted) return "retirement_uncertain";
  if (!view.fresh) {
    const known = view.observation.state.kind === "applied" ||
      view.observation.state.kind === "refused";
    return view.applyAttempted && !known ? "unknown" : "unavailable";
  }
  if (view.observation.pending) return "pending";
  const state = view.observation.state;
  if (state.kind === "refused") return state.reason;
  if (state.kind === "applied") return "applied";
  if (view.applyAttempted && state.kind === "prepared") return "unknown";
  if (state.kind === "prepared" || state.kind === "pending") return state.kind;
  return "unknown";
}

export function createSynchronizationProjection(context: AbortSignal, source: {
  subscribe(listener: () => void): () => void;
  revision(): number;
  entries(): ReadonlyMap<OwnedCodeBuffer, { target: BufferTarget }>;
}): CodeBufferSynchronizations {
  let ordinal = 0;
  let revision = -1;
  let cached: SynchronizationsView | undefined;
  const handles = new WeakMap<
    OwnedSynchronization,
    { handle: SynchronizationHandle; ordinal: number }
  >();
  const owners = new WeakMap<
    SynchronizationHandle,
    { owner: OwnedCodeBuffer; operation: OwnedSynchronization }
  >();
  const original = (handle: SynchronizationHandle) => {
    if (context.aborted) throw new BufferClientError("context_lost");
    const entry = owners.get(handle);
    if (
      !entry || !source.entries().has(entry.owner) ||
      entry.owner.synchronization() !== entry.operation
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
      const rows: SynchronizationRow[] = [];
      for (const [owner, entry] of source.entries()) {
        const operation = owner.synchronization();
        if (!operation) continue;
        let identity = handles.get(operation);
        if (!identity) {
          if (ordinal >= Number.MAX_SAFE_INTEGER) {
            throw new BufferClientError("capacity");
          }
          identity = {
            handle: Object.freeze({}) as SynchronizationHandle,
            ordinal: ++ordinal,
          };
          handles.set(operation, identity);
          owners.set(identity.handle, { owner, operation });
        }
        const view = operation.view();
        rows.push(Object.freeze({
          ...identity,
          target: context.aborted ? undefined : entry.target,
          content: context.aborted ? undefined : view.observation.content,
          status: status(view),
          canApply: view.canApply,
          canRetire: view.canRetire,
          canInspect: view.canInspect,
        }));
      }
      revision = source.revision();
      cached = Object.freeze({
        contextLost: context.aborted,
        rows: Object.freeze(rows),
      });
      return cached;
    },
    async inspect(handle: SynchronizationHandle, observer?: AbortSignal) {
      await original(handle).observe(observer);
    },
    preview(handle: SynchronizationHandle, action: SynchronizationAction) {
      return original(handle).preview(action);
    },
    isCurrent(
      handle: SynchronizationHandle,
      token: SynchronizationConfirmation,
    ) {
      try {
        return original(handle).isCurrent(token);
      } catch {
        return false;
      }
    },
    async confirm(
      handle: SynchronizationHandle,
      token: SynchronizationConfirmation,
      observer?: AbortSignal,
    ) {
      await original(handle).confirm(token, observer);
    },
  });
}
