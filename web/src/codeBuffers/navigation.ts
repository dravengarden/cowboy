/** One original-source acquisition. Observer cancellation cannot replay effects. */
import { BufferClientError, type Failure, requireValue } from "./protocol.ts";
import {
  decodeNavigation,
  type NavigationSnapshot,
} from "./navigationProtocol.ts";
import { observePromise } from "./transport.ts";

export interface NavigationView {
  readonly observation: NavigationSnapshot;
  readonly fresh: boolean;
  readonly busy: boolean;
  readonly contextLost: boolean;
  readonly executeAttempted: boolean;
  readonly releaseAttempted: boolean;
  readonly failure: Failure | undefined;
  readonly canExecute: boolean;
  readonly canRelease: boolean;
  readonly canInspect: boolean;
}
export interface OwnedNavigation {
  view(): NavigationView;
  execute(observer?: AbortSignal): Promise<NavigationSnapshot>;
  observe(observer?: AbortSignal): Promise<NavigationSnapshot>;
  /** Group release only, not buffer release, rollback or native close proof. */
  release(observer?: AbortSignal): Promise<NavigationSnapshot>;
}

/** Internal constructor used only by the original buffer's admitted preparation.
 * Shared ports retain that buffer's job fence and actual core identity lifetime.
 */
export function ownNavigation(prepared: NavigationSnapshot, ports: {
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
}): OwnedNavigation {
  let last = prepared;
  let fresh = true;
  let ended = false;
  let executed = false;
  let releasing = false;
  let failure: Failure | undefined;
  const available = () => !ended && !ports.context.aborted && !ports.busy();
  const executeAllowed = () =>
    available() && fresh && !last.pending &&
    !ports.closing() && !executed && !releasing && last.state === "prepared";
  const releaseAllowed = () =>
    available() && fresh && !last.pending &&
    !releasing &&
    (last.state === "retained" || !executed && last.state === "prepared");
  const run = (method: "PUT" | "GET" | "DELETE", observer?: AbortSignal) => {
    ports.check(observer);
    if (ports.busy()) throw new BufferClientError("busy");
    if (
      ended || method === "PUT" && !executeAllowed() ||
      method === "DELETE" && !releaseAllowed()
    ) throw new BufferClientError("state");
    return observePromise(
      ports.perform(async () => {
        if (method === "PUT") executed = true;
        if (method === "DELETE") releasing = true;
        fresh = false;
        ports.changed();
        try {
          const reply = await ports.request(method);
          const next = decodeNavigation(
            reply.value,
            reply.status,
            prepared.sourceResourceId,
            prepared,
            prepared.navigationId,
          );
          if (last.state === "retained" || last.state === "release_unknown") {
            requireValue(
              JSON.stringify(next.locations) === JSON.stringify(last.locations),
            );
          }
          // Known acquired targets never become an inert/empty preparation again.
          if (!executed) requireValue(next.locations.length === 0);
          if (last.state === "unknown") {
            requireValue(next.state === "unknown" || next.state === "retained");
          } else if (last.state === "retained") {
            requireValue(
              next.state === "retained" ||
                releasing &&
                  (next.state === "release_unknown" ||
                    next.state === "released"),
            );
          } else if (last.state === "release_unknown") {
            requireValue(
              next.state === "release_unknown" || next.state === "released",
            );
          } else if (!executed) {
            requireValue(
              next.state === "prepared" || next.state === "expired" ||
                releasing &&
                  (next.state === "release_unknown" ||
                    next.state === "released"),
            );
          } else {
            requireValue(
              next.state === "unknown" || next.state === "retained" ||
                next.state === "expired" ||
                next.state === "prepared" && next.pending,
            );
          }
          if (method === "DELETE") {
            requireValue(
              next.pending || next.state === "release_unknown" ||
                next.state === "released" || next.state === "expired",
            );
            // Only an exact no-admission acknowledgement can permit a later DELETE.
            // An unknown or lost Release remains query-only, including after 202.
            if (
              next.pending && next.state === last.state &&
              next.state !== "release_unknown"
            ) {
              releasing = false;
            }
          }
          if (ports.context.aborted) {
            throw new BufferClientError("context_lost");
          }
          last = next;
          fresh = true;
          failure = undefined;
          if (next.state === "released" || next.state === "expired") {
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
        executeAttempted: executed,
        releaseAttempted: releasing,
        failure,
        canExecute: executeAllowed(),
        canRelease: releaseAllowed(),
        canInspect: available(),
      }),
    async execute(observer?: AbortSignal) {
      return await run("PUT", observer);
    },
    async observe(observer?: AbortSignal) {
      return await run("GET", observer);
    },
    async release(observer?: AbortSignal) {
      return await run("DELETE", observer);
    },
  });
}
