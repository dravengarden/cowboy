/** Core browser continuation owner. No React mount, path retry, saved JSON or
 * browser abort can replace the original resource or prove its release.
 */
import { decodeObservation } from "./observations.ts";
import { createCleanupMonitor } from "./cleanup.ts";
import {
  type CapturedContent,
  capturedIdentity,
  type ContentIdentity,
  type ContentKind,
  type ContentObservation,
  type ContentQueries,
  contentRequest,
  decodeContentObservation,
} from "./content.ts";
import {
  BufferClientError,
  decodeSnapshot,
  type Failure,
  type Observation,
  type Point,
  type ReadKind,
  requireValue,
  type ResourceId,
  type Snapshot,
  text,
} from "./protocol.ts";
import {
  createTransport,
  observePromise,
  type TransportOptions,
} from "./transport.ts";
import {
  type OwnedSynchronization,
  ownSynchronization,
} from "./synchronization.ts";
import { decodeSynchronization } from "./synchronizationProtocol.ts";
import { readCompleteText, textIdentity, type TextRead } from "./text.ts";
import { type OwnedNavigation, ownNavigation } from "./navigation.ts";
import type { DestinationReservation } from "./navigationDestinations.ts";
import {
  decodeNavigation,
  type NavigationKind,
  type NavigationLocation,
  navigationRequest,
} from "./navigationProtocol.ts";

type Job =
  | "prepare"
  | "open"
  | "observe"
  | "read"
  | "release"
  | "synchronize"
  | "navigate";
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
  readonly cleaning: boolean;
  /** An unacknowledged DELETE is observation-only, never a new release intent. */
  readonly releaseAttempted: boolean;
  readonly contextLost: boolean;
  readonly failure: Failure | undefined;
  /** A synchronization must be explicitly retired before releasing this owner. */
  readonly synchronizing: boolean;
  /** Original navigation must end explicitly before source reads or release. */
  readonly navigating: boolean;
  /** Capacity is owned, but only the original navigation can provide its ID. */
  readonly handingOff: boolean;
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
  /** Conditional read only. End the observer when the displayed snapshot changes. */
  readContent<Q extends ContentQueries[ContentKind]>(
    content: CapturedContent,
    query: Q,
    observer?: AbortSignal,
  ): Promise<ContentObservation<Q["kind"]>>;
  /** Native text only; no partial display, automatic retry, reload or path read. */
  readText(content: ContentIdentity, observer?: AbortSignal): Promise<TextRead>;
  /** Effect-free native preparation, never Apply. Requires an originally opened owner. */
  prepareSynchronization(
    content: CapturedContent,
    observer?: AbortSignal,
  ): Promise<OwnedSynchronization>;
  synchronization(): OwnedSynchronization | undefined;
  /** Effect-free preparation only; Execute is a separate original-owner action. */
  prepareNavigation(
    content: CapturedContent,
    position: Point,
    query: NavigationKind,
    observer?: AbortSignal,
  ): Promise<OwnedNavigation>;
  navigation(): OwnedNavigation | undefined;
  /** One bounded cleanup pass. Retained is NOT closed or automatically retried. */
  close(): Promise<CloseResult>;
}

const MAX_OWNERS = 64;
const MAX_NAVIGATIONS = 32;

export function createOwnedCodeBuffers(options: TransportOptions) {
  const context = options.context;
  const transport = createTransport({ ...options, context });
  const monitor = createCleanupMonitor(context);
  let navigations = 0;
  const reserveNavigation = () => {
    if (navigations >= MAX_NAVIGATIONS) throw new BufferClientError("capacity");
    ++navigations;
    let reserved = true;
    return () => {
      if (!reserved) return;
      reserved = false;
      --navigations;
    };
  };
  const allocate = (
    target: BufferTarget,
    destination = false,
  ): DestinationReservation => {
    transport.check();
    if (monitor.size() >= MAX_OWNERS) throw new BufferClientError("capacity");
    const captured = Object.freeze({
      sessionId: text(target.sessionId, 128),
      path: text(target.path, 4096),
    });
    if (!captured.sessionId || !captured.path) {
      throw new BufferClientError("protocol");
    }
    const entry = createOwner(
      captured,
      context,
      transport,
      () => monitor.retire(entry.owner),
      monitor.changed,
      reserveNavigation,
      destination,
      (location) =>
        allocate({ sessionId: captured.sessionId, path: location.path }, true),
      (id) =>
        requireValue(
          !monitor.retained().some((owner) =>
            owner !== entry.owner && owner.view().resourceId === id
          ),
        ),
    );
    monitor.add(entry.owner, captured);
    return entry;
  };
  return Object.freeze({
    cleanup: monitor.store,
    synchronizations: monitor.synchronizations,
    navigations: monitor.navigations,
    /** No restore(id), import, serialization, LRU or automatic account adoption. */
    reserve(target: BufferTarget): OwnedCodeBuffer {
      return allocate(target).owner;
    },
    retained(): readonly OwnedCodeBuffer[] {
      return monitor.retained();
    },
  });
}

function createOwner(
  target: BufferTarget,
  context: AbortSignal,
  transport: ReturnType<typeof createTransport>,
  retire: () => void,
  changed: () => void,
  reserveNavigation: () => () => void,
  destination: boolean,
  reserveDestination: (
    location: NavigationLocation,
  ) => DestinationReservation,
  uniqueDestination: (id: ResourceId) => void,
): DestinationReservation {
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
  let synchronization: OwnedSynchronization | undefined;
  let navigation: OwnedNavigation | undefined;
  let handingOff = destination;
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
    changed();
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
    changed();
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
      changed();
    }
  };
  const snapshot = async (method: "PUT" | "GET" | "DELETE") => {
    const current = resource();
    const reply = await transport.request(`/${current}`, method, {}, 1024);
    return accept(decodeSnapshot(reply.value, reply.status, current));
  };
  const observe = async (observer?: AbortSignal): Promise<Snapshot> => {
    check(observer);
    if (synchronization || navigation || handingOff) unavailable("state");
    if (phase === "released" && last) return Promise.resolve(last);
    resource();
    return observePromise(perform("observe", () => snapshot("GET")), observer);
  };
  const cleanupPass = async (): Promise<CloseResult> => {
    // Detach immediately, drain the owned continuation, then decide using its
    // result. This queues a LOCAL cleanup pass, not a server DELETE from a 202.
    await job?.settled;
    if (handingOff) return Object.freeze({ kind: "retained", owner });
    if (!id) {
      phase = "unopened";
      retire();
      return Object.freeze({ kind: "unopened" });
    }
    if (phase === "released") {
      return Object.freeze({ kind: "released", resourceId: id });
    }
    // No hidden synchronization or navigation actions during view cleanup.
    if (synchronization || navigation) {
      return Object.freeze({ kind: "retained", owner });
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
        cleaning: cleanup !== undefined,
        releaseAttempted: releaseSent,
        contextLost: context.aborted,
        failure,
        synchronizing: !!synchronization || job?.kind === "synchronize",
        navigating: !!navigation || job?.kind === "navigate",
        handingOff,
      }),
    async prepare(observer?: AbortSignal) {
      check(observer);
      if (destination || prepared || closing || phase !== "reserved") {
        unavailable("state");
      }
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
        closing || synchronization || navigation || releaseSent || !fresh ||
        last?.state !== "open" ||
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
    async readContent<Q extends ContentQueries[ContentKind]>(
      content: CapturedContent,
      query: Q,
      observer?: AbortSignal,
    ): Promise<ContentObservation<Q["kind"]>> {
      check(observer);
      if (
        closing || synchronization || navigation || releaseSent || !fresh ||
        last?.state !== "open" ||
        last.pending
      ) unavailable("state");
      const current = resource();
      const request = contentRequest(content, query);
      return observePromise(
        perform("read", async () => {
          const reply = await transport.request(
            `/${current}/read`,
            "POST",
            request,
            2 * 1024 * 1024,
          );
          if (reply.status !== 200) unavailable("protocol");
          const result = decodeContentObservation<Q["kind"]>(
            reply.value,
            current,
            request,
          );
          transport.check();
          if (closing || observer?.aborted) unavailable("cancelled");
          return result;
        }),
        observer,
      );
    },
    async readText(
      content: ContentIdentity,
      observer?: AbortSignal,
    ): Promise<TextRead> {
      check(observer);
      if (
        closing || synchronization || navigation || releaseSent || !fresh ||
        last?.state !== "open" || last.pending
      ) unavailable("state");
      const current = resource();
      const expected = textIdentity(content);
      return observePromise(
        perform("read", () =>
          readCompleteText(
            current,
            expected,
            (request) =>
              transport.request(
                `/${current}/read`,
                "POST",
                request,
                512 * 1024,
              ),
            () => {
              transport.check();
              if (closing || observer?.aborted) unavailable("cancelled");
            },
          )),
        observer,
      );
    },
    async prepareSynchronization(
      content: CapturedContent,
      observer?: AbortSignal,
    ) {
      check(observer);
      if (
        synchronization || navigation || closing || releaseSent || !openSent ||
        !fresh ||
        last?.state !== "open" || last.pending
      ) unavailable("state");
      const identity = capturedIdentity(content);
      // The native disk-sync primitive refuses BOM input; never normalize it.
      requireValue(!content.text.startsWith("\uFEFF"));
      const current = resource();
      return observePromise(
        perform("synchronize", async () => {
          fresh = false; // old buffer observations cannot authorize subsequent reads
          const reply = await transport.request(
            `/${current}/synchronizations`,
            "POST",
            {
              purpose: "refresh_from_disk",
              content: identity,
            },
            16 * 1024,
          );
          const prepared = decodeSynchronization(
            reply.value,
            reply.status,
            current,
            identity,
          );
          requireValue(prepared.state.kind === "prepared" && !prepared.pending);
          transport.check();
          const owned = ownSynchronization(prepared, {
            context,
            check,
            busy: () => !!job || !!cleanup,
            closing: () => closing,
            perform: (effect) => perform("synchronize", effect),
            request: (method) =>
              transport.request(
                `/${prepared.operationId}`,
                method,
                {},
                16 * 1024,
                "buffer-synchronizations",
              ),
            changed,
            retired: () => {
              if (synchronization === owned) synchronization = undefined;
              fresh = false; // a new explicit original-ID observation is required
            },
          });
          synchronization = owned;
          changed();
          return owned;
        }),
        observer,
      );
    },
    synchronization: () => synchronization,
    async prepareNavigation(
      content: CapturedContent,
      position: Point,
      query: NavigationKind,
      observer?: AbortSignal,
    ) {
      check(observer);
      if (
        navigation || synchronization || closing || releaseSent || !openSent ||
        !fresh ||
        last?.state !== "open" || last.pending
      ) unavailable("state");
      const request = navigationRequest(content, position, query);
      const current = resource();
      // Count pending preparations too. Never evict or expire an uncertain effect.
      const free = reserveNavigation();
      return observePromise(
        perform("navigate", async () => {
          fresh = false;
          try {
            const reply = await transport.request(
              `/${current}/navigations`,
              "POST",
              request,
              2 * 1024 * 1024,
            );
            const prepared = decodeNavigation(
              reply.value,
              reply.status,
              current,
              request,
            );
            requireValue(prepared.state === "prepared" && !prepared.pending);
            transport.check();
            const owned = ownNavigation(prepared, {
              context,
              check,
              busy: () => !!job || !!cleanup,
              closing: () => closing,
              perform: (effect) => perform("navigate", effect),
              request: (method) =>
                transport.request(
                  `/${prepared.navigationId}`,
                  method,
                  {},
                  2 * 1024 * 1024,
                  "navigations",
                ),
              changed,
              reserveDestination,
              prepareDestination: (destination, content) =>
                transport.request(
                  `/${prepared.navigationId}/destinations`,
                  "POST",
                  { destination, content },
                  2 * 1024 * 1024,
                  "navigations",
                ),
              retired: () => {
                if (navigation === owned) navigation = undefined;
                free();
                fresh = false;
              },
            });
            navigation = owned;
            changed();
            return owned;
          } catch (error) {
            free(); // unobserved preparation is effect-free; never Execute here
            throw error;
          }
        }),
        observer,
      );
    },
    navigation: () => navigation,
    close() {
      closing = true;
      if (cleanup) return cleanup;
      cleanup = cleanupPass().finally(() => {
        cleanup = undefined;
        changed();
      });
      changed();
      return cleanup;
    },
  });
  const validate = (next: ResourceId) => {
    transport.check();
    requireValue(destination);
    if (id) requireValue(id === next);
    else {
      requireValue(handingOff && phase === "reserved");
      uniqueDestination(next);
    }
  };
  return {
    owner,
    validate,
    adopt(next) {
      validate(next);
      if (id) return; // historical group evidence cannot reset an opened/closed child
      id = next;
      prepared = true;
      handingOff = false;
      phase = "retained";
      accept(
        Object.freeze({
          apiVersion: 1,
          resourceId: next,
          state: "prepared",
          pending: false,
        }),
      );
    },
    abandon() {
      requireValue(destination && !id);
      handingOff = false;
      phase = "unopened"; // only an inert reservation, never an acquired buffer
      retire();
      changed();
    },
  };
}
