/** Core browser continuation owner. No React mount, path retry, saved JSON or
 * browser abort can replace the original resource or prove its release.
 */
import { decodeObservation } from "./observations.ts";
import {
  BufferClientError,
  decodeSnapshot,
  type Failure,
  type Observation,
  type ReadKind,
  type ResourceId,
  type Snapshot,
  text,
} from "./protocol.ts";
import {
  createTransport,
  observePromise,
  type TransportOptions,
} from "./transport.ts";

type Job = "prepare" | "open" | "observe" | "read" | "release";
export interface BufferTarget {
  readonly sessionId: string;
  readonly path: string;
}
export interface OwnerView {
  readonly phase: "reserved" | "retained" | "unopened" | "released";
  readonly resourceId: ResourceId | undefined;
  /** Last evidence only. A failed request makes it unusable for new reads. */
  readonly observation: Snapshot | undefined;
  readonly fresh: boolean;
  readonly busy: Job | undefined;
  readonly closing: boolean;
  readonly contextLost: boolean;
  readonly failure: Failure | undefined;
}
export type CloseResult =
  | { readonly kind: "unopened" }
  | { readonly kind: "released"; readonly resourceId: ResourceId }
  | { readonly kind: "retained"; readonly owner: OwnedCodeBuffer };
export interface OwnedCodeBuffer {
  view(): OwnerView;
  prepare(observer?: AbortSignal): Promise<Snapshot>;
  open(observer?: AbortSignal): Promise<Snapshot>;
  observe(observer?: AbortSignal): Promise<Snapshot>;
  read<K extends ReadKind>(
    kind: K,
    observer?: AbortSignal,
  ): Promise<Observation<K>>;
  /** One bounded cleanup pass. Retained is NOT closed or automatically retried. */
  close(): Promise<CloseResult>;
}

const MAX_OWNERS = 64;

export function createOwnedCodeBuffers(options: TransportOptions) {
  const context = options.context;
  const transport = createTransport({ ...options, context });
  const retained = new Set<OwnedCodeBuffer>();
  return Object.freeze({
    /** No restore(id), import, serialization, LRU or automatic account adoption. */
    reserve(target: BufferTarget): OwnedCodeBuffer {
      transport.check();
      if (retained.size >= MAX_OWNERS) throw new BufferClientError("capacity");
      const captured = Object.freeze({
        sessionId: text(target.sessionId, 128),
        path: text(target.path, 4096),
      });
      if (!captured.sessionId || !captured.path) {
        throw new BufferClientError("protocol");
      }
      const owner = createOwner(
        captured,
        context,
        transport,
        () => retained.delete(owner),
      );
      retained.add(owner);
      return owner;
    },
    retained(): readonly OwnedCodeBuffer[] {
      return Object.freeze([...retained]);
    },
  });
}

function createOwner(
  target: BufferTarget,
  context: AbortSignal,
  transport: ReturnType<typeof createTransport>,
  retire: () => void,
): OwnedCodeBuffer {
  let phase: OwnerView["phase"] = "reserved";
  let id: ResourceId | undefined;
  let last: Snapshot | undefined;
  let fresh = false;
  let failure: Failure | undefined;
  let closing = false;
  let prepared = false;
  let openSent = false;
  let releaseSent = false;
  let job: { kind: Job; settled: Promise<void> } | undefined;
  let cleanup: Promise<CloseResult> | undefined;
  function unavailable(kind: Failure): never {
    throw new BufferClientError(kind);
  }
  const check = (observer?: AbortSignal) => {
    transport.check();
    if (observer?.aborted) unavailable("cancelled");
    if (job) unavailable("busy");
  };
  const resource = (): ResourceId => {
    if (!id) unavailable("state");
    return id;
  };
  const accept = (snapshot: Snapshot) => {
    transport.check();
    last = snapshot;
    fresh = true;
    failure = undefined;
    if (snapshot.state === "released") {
      phase = "released";
      retire();
    }
    return snapshot;
  };
  const perform = async <T>(
    kind: Job,
    effect: () => Promise<T>,
  ): Promise<T> => {
    check();
    let settle!: () => void;
    job = {
      kind,
      settled: new Promise<void>((resolve) => {
        settle = resolve;
      }),
    };
    try {
      const result = await effect();
      transport.check();
      return result;
    } catch (error) {
      fresh = false;
      failure = error instanceof BufferClientError ? error.kind : "transport";
      throw error instanceof BufferClientError
        ? error
        : new BufferClientError("transport");
    } finally {
      job = undefined;
      settle();
    }
  };
  const snapshot = async (method: "PUT" | "GET" | "DELETE") => {
    const current = resource();
    const reply = await transport.request(`/${current}`, method, {}, 1024);
    return accept(decodeSnapshot(reply.value, reply.status, current));
  };
  const observe = async (observer?: AbortSignal): Promise<Snapshot> => {
    check(observer);
    if (phase === "released" && last) return Promise.resolve(last);
    resource();
    return observePromise(perform("observe", () => snapshot("GET")), observer);
  };
  const cleanupPass = async (): Promise<CloseResult> => {
    // Detach immediately, drain the owned continuation, then decide using its
    // result. This queues a LOCAL cleanup pass, not a server DELETE from a 202.
    await job?.settled;
    if (!id) {
      phase = "unopened";
      retire();
      return Object.freeze({ kind: "unopened" });
    }
    if (phase === "released") {
      return Object.freeze({ kind: "released", resourceId: id });
    }
    try {
      transport.check();
      if (!fresh || last?.pending || last?.state === "unknown" || releaseSent) {
        await observe();
      }
      if (last?.state === "released") {
        return Object.freeze({ kind: "released", resourceId: id });
      }
      if (
        fresh && last && !last.pending && last.state !== "unknown" &&
        !releaseSent
      ) {
        await perform("release", async () => {
          releaseSent = true; // uncertainty never rearms a mutation
          const result = await snapshot("DELETE");
          // A valid 202 proves THIS request was not queued/admitted. An
          // explicit later close must observe completion before trying again.
          if (result.pending) releaseSent = false;
          return result;
        });
      }
    } catch (error) {
      fresh = false;
      failure = error instanceof BufferClientError ? error.kind : "transport";
    }
    return last?.state === "released" && fresh
      ? Object.freeze({ kind: "released", resourceId: id })
      : Object.freeze({ kind: "retained", owner });
  };
  const owner: OwnedCodeBuffer = Object.freeze({
    view: () =>
      Object.freeze({
        phase,
        resourceId: id,
        observation: last,
        fresh: fresh && !context.aborted,
        busy: job?.kind,
        closing,
        contextLost: context.aborted,
        failure,
      }),
    async prepare(observer?: AbortSignal) {
      check(observer);
      if (prepared || closing || phase !== "reserved") unavailable("state");
      prepared = true;
      return observePromise(
        perform("prepare", async () => {
          try {
            const reply = await transport.request("", "POST", target, 1024);
            const result = decodeSnapshot(reply.value, reply.status);
            if (result.state !== "prepared" || result.pending) {
              unavailable("protocol");
            }
            transport.check();
            id = result.resourceId;
            phase = "retained";
            return accept(result);
          } catch (error) {
            phase = "unopened";
            retire(); // only effect-free, unobserved preparation can expire
            throw error;
          }
        }),
        observer,
      );
    },
    async open(observer?: AbortSignal) {
      check(observer);
      if (
        closing || openSent || !fresh || last?.state !== "prepared" ||
        last.pending
      ) unavailable("state");
      openSent = true;
      return observePromise(perform("open", () => snapshot("PUT")), observer);
    },
    observe,
    async read<K extends ReadKind>(
      kind: K,
      observer?: AbortSignal,
    ): Promise<Observation<K>> {
      check(observer);
      if (kind !== "language" && kind !== "symbols") unavailable("protocol");
      if (
        closing || releaseSent || !fresh || last?.state !== "open" ||
        last.pending
      ) unavailable("state");
      const current = resource();
      return observePromise(
        perform("read", async () => {
          const reply = await transport.request(`/${current}/read`, "POST", {
            kind,
          }, 2 * 1024 * 1024);
          if (reply.status !== 200) unavailable("protocol");
          const result = decodeObservation(reply.value, current, kind);
          transport.check();
          // Ending a view discards results, but does not cancel the borrow or
          // redirect cleanup to the next view's path/context.
          if (closing || observer?.aborted) unavailable("cancelled");
          return result;
        }),
        observer,
      );
    },
    close() {
      closing = true;
      if (cleanup) return cleanup;
      cleanup = cleanupPass().finally(() => {
        cleanup = undefined;
      });
      return cleanup;
    },
  });
  return owner;
}
