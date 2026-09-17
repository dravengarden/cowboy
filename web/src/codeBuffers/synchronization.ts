import { BufferClientError, type Failure, requireValue } from "./protocol.ts";
import {
  decodeSynchronization,
  type SynchronizationSnapshot,
} from "./synchronizationProtocol.ts";
import { observePromise } from "./transport.ts";

export type SynchronizationAction = "apply" | "retire";
declare const confirmation: unique symbol;
export interface SynchronizationConfirmation {
  readonly [confirmation]: true;
}
export interface SynchronizationView {
  readonly observation: SynchronizationSnapshot;
  readonly fresh: boolean;
  readonly busy: boolean;
  readonly contextLost: boolean;
  readonly applyAttempted: boolean;
  readonly retireAttempted: boolean;
  readonly failure: Failure | undefined;
  readonly canApply: boolean;
  readonly canRetire: boolean;
  readonly canInspect: boolean;
}
export interface OwnedSynchronization {
  view(): SynchronizationView;
  observe(observer?: AbortSignal): Promise<SynchronizationSnapshot>;
  /** Local preview only. Any intervening operation invalidates this token. */
  preview(action: SynchronizationAction): SynchronizationConfirmation;
  isCurrent(token: SynchronizationConfirmation): boolean;
  confirm(
    token: SynchronizationConfirmation,
    observer?: AbortSignal,
  ): Promise<SynchronizationSnapshot>;
}

/** Only the original buffer owner constructs this continuation after preparation.
 * The ports share its admission fence; this is not an ID-based public factory.
 */
export function ownSynchronization(
  prepared: SynchronizationSnapshot,
  ports: {
    context: AbortSignal;
    check(observer?: AbortSignal): void;
    busy(): boolean;
    closing(): boolean;
    perform<T>(effect: () => Promise<T>): Promise<T>;
    request(
      method: "PUT" | "GET" | "DELETE",
    ): Promise<{ value: unknown; status: number }>;
    changed(): void;
    retired(): void;
  },
): OwnedSynchronization {
  let last = prepared;
  let fresh = true;
  let ended = false;
  let applied = false;
  let retiring = false;
  let revision = 0;
  let failure: Failure | undefined;
  const confirmations = new WeakMap<SynchronizationConfirmation, {
    action: SynchronizationAction;
    revision: number;
  }>();
  const terminal = () =>
    last.state.kind === "applied" || last.state.kind === "refused";
  const available = () => !ended && !ports.context.aborted && !ports.busy();
  const allowed = (action: SynchronizationAction) =>
    available() && fresh && !last.pending && !retiring &&
    (action === "apply"
      ? !ports.closing() && !applied && last.state.kind === "prepared"
      : action === "retire" &&
        (terminal() || !applied && last.state.kind === "prepared"));
  const current = (token: SynchronizationConfirmation) => {
    const entry = confirmations.get(token);
    return !!entry && entry.revision === revision && allowed(entry.action);
  };
  const run = (
    method: "PUT" | "GET" | "DELETE",
    observer?: AbortSignal,
  ) => {
    ports.check(observer);
    if (ended) throw new BufferClientError("state");
    ++revision;
    return observePromise(
      ports.perform(async () => {
        // Consume before transport. An observer abort or HTTP error never rearms Apply.
        if (method === "PUT") applied = true;
        if (method === "DELETE") retiring = true;
        fresh = false;
        ports.changed();
        try {
          const reply = await ports.request(method);
          const next = decodeSynchronization(
            reply.value,
            reply.status,
            prepared.resourceId,
            prepared.content,
            prepared.operationId,
          );
          // A terminal result is immutable. Retired is evidence only after an
          // inert preparation or an explicitly retired known terminal result.
          if (terminal()) {
            requireValue(
              JSON.stringify(next.state) === JSON.stringify(last.state) ||
                retiring && next.state.kind === "retired",
            );
          } else if (applied) {
            requireValue(next.state.kind !== "retired");
            // A valid 202 says no new job was queued, but does not renew our
            // one-use confirmation. A later Service expiry proves inertness.
            requireValue(next.state.kind !== "prepared" || next.pending);
          } else {
            requireValue(
              next.state.kind === "prepared" || next.state.kind === "retired" ||
                next.state.kind === "expired",
            );
          }
          if (method === "DELETE") {
            requireValue(
              next.pending || next.state.kind === "retired" ||
                next.state.kind === "expired",
            );
            // Only this exact no-admission acknowledgement allows a new,
            // separately confirmed retirement; a lost response is query-only.
            if (next.pending) retiring = false;
          }
          if (ports.context.aborted) {
            throw new BufferClientError("context_lost");
          }
          last = next;
          fresh = true;
          failure = undefined;
          if (next.state.kind === "retired" || next.state.kind === "expired") {
            ended = true;
            ports.retired();
          }
          return next;
        } catch (error) {
          failure = error instanceof BufferClientError
            ? error.kind
            : "transport";
          throw error;
        } finally {
          ports.changed();
        }
      }),
      observer,
    );
  };
  return Object.freeze({
    view: () =>
      Object.freeze({
        observation: last,
        fresh: fresh && !ports.context.aborted,
        busy: ports.busy(),
        contextLost: ports.context.aborted,
        applyAttempted: applied,
        retireAttempted: retiring,
        failure,
        canApply: allowed("apply"),
        canRetire: allowed("retire"),
        canInspect: available(),
      }),
    async observe(observer?: AbortSignal) {
      return await run("GET", observer);
    },
    preview(action: SynchronizationAction) {
      ports.check();
      if (!allowed(action)) throw new BufferClientError("state");
      const token = Object.freeze({}) as SynchronizationConfirmation;
      confirmations.set(token, { action, revision });
      return token;
    },
    isCurrent: current,
    async confirm(token: SynchronizationConfirmation, observer?: AbortSignal) {
      ports.check(observer);
      const entry = confirmations.get(token);
      if (!entry || !current(token)) throw new BufferClientError("state");
      confirmations.delete(token);
      return await run(entry.action === "apply" ? "PUT" : "DELETE", observer);
    },
  });
}
