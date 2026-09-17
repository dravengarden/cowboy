/** Review orchestration over the ONE core owner, not a second resource registry.
 * Read observers may leave; admitted native work must drain before the next read.
 */
import type {
  CapturedContent,
  ContentKind,
  ContentQueries,
} from "../../codeBuffers/content.ts";
import type {
  BufferTarget,
  createOwnedCodeBuffers,
  OwnedCodeBuffer,
} from "../../codeBuffers/owner.ts";
import { BufferClientError } from "../../codeBuffers/protocol.ts";
import { observePromise } from "../../codeBuffers/transport.ts";

export type ReviewBufferMode = "legacy" | "owned" | "unavailable";
export interface ReviewBufferSource {
  ready(
    observer?: AbortSignal,
  ): Promise<ReturnType<typeof createOwnedCodeBuffers>>;
}

/** Unknown explicit formats fail closed. Once owned, this view never downgrades. */
export function reviewBufferMode(
  previous: ReviewBufferMode | undefined,
  value: unknown,
): ReviewBufferMode {
  if (previous === "owned") return previous;
  if (value === "owned") return "owned";
  if (value === undefined || value === "legacy") return "legacy";
  return "unavailable";
}

export function createReviewBuffer(
  source: ReviewBufferSource,
  input: BufferTarget,
) {
  const target = Object.freeze({ ...input });
  const lifetime = new AbortController();
  let owner: OwnedCodeBuffer | undefined;
  let queued = 0;
  let tail: Promise<unknown> = Promise.resolve();
  let refreshObserver:
    | { signal: AbortSignal; listener: () => void }
    | undefined;
  const clearRefreshObserver = () => {
    if (refreshObserver) {
      refreshObserver.signal.removeEventListener(
        "abort",
        refreshObserver.listener,
      );
    }
    refreshObserver = undefined;
  };
  const close = () => {
    clearRefreshObserver();
    lifetime.abort();
    // Synchronously fence Apply/open/read, including a prepare still in flight.
    // The core registry retains unresolved cleanup for Settings.
    return owner?.close() ?? Promise.resolve({ kind: "unopened" as const });
  };
  const check = (signal?: AbortSignal) => {
    if (lifetime.signal.aborted || signal?.aborted) {
      throw new BufferClientError("cancelled");
    }
  };
  // Construct only in a committed effect, never render. No retries or timers.
  const ready = (async () => {
    const registry = await source.ready(lifetime.signal);
    check();
    owner = registry.reserve(target);
    const prepared = await owner.prepare();
    check();
    if (prepared.pending || prepared.state !== "prepared") {
      throw new BufferClientError("state");
    }
    const opened = await owner.open();
    check();
    if (opened.pending || opened.state !== "open") {
      throw new BufferClientError("state");
    }
    return owner;
  })();
  // An unobserved initialization failure must not become an unhandled rejection.
  void ready.catch(() => undefined);

  function run<T>(
    action: (owner: OwnedCodeBuffer) => Promise<T>,
    signal: AbortSignal,
  ): Promise<T> {
    try {
      check(signal);
      if (queued >= 8) throw new BufferClientError("capacity");
    } catch (error) {
      return Promise.reject(error);
    }
    queued++;
    const task = tail.then(async () => {
      check(signal);
      const original = await ready;
      check(signal);
      // Do NOT pass the view's signal into this owned borrow: queue exclusion
      // lasts until the actual continuation drains, not until a waiter leaves.
      const result = await action(original);
      check(signal);
      return result;
    }).finally(() => {
      queued--;
    });
    tail = task.catch(() => undefined);
    return observePromise(task, AbortSignal.any([lifetime.signal, signal]));
  }

  return Object.freeze({
    read<Q extends ContentQueries[ContentKind]>(
      content: CapturedContent,
      query: Q,
      signal: AbortSignal,
      reconcile = false,
    ) {
      // Capture mutable caller positions before waiting behind an existing read.
      const capturedQuery = structuredClone(query);
      return run(async (original) => {
        // Only the explicit Check action requests reconciliation. A read or
        // mismatch never reloads, reopens, retires synchronization or retries.
        if (reconcile && !original.view().fresh) await original.observe();
        check(signal);
        return original.readContent(content, capturedQuery);
      }, signal);
    },
    prepareRefresh(content: CapturedContent, signal: AbortSignal) {
      return run(async (original) => {
        if (original.synchronization()) throw new BufferClientError("state");
        clearRefreshObserver();
        const listener = () => {
          // Abandoning the displayed text revokes Apply even when Settings keeps
          // a confirmation or preparation has not returned its ID yet.
          if (
            original.synchronization() || original.view().busy === "synchronize"
          ) void close();
        };
        refreshObserver = { signal, listener };
        signal.addEventListener("abort", listener, { once: true });
        try {
          return await original.prepareSynchronization(content);
        } catch (error) {
          if (!original.synchronization()) clearRefreshObserver();
          throw error;
        }
      }, signal);
    },
    close,
  });
}
export type ReviewBuffer = ReturnType<typeof createReviewBuffer>;
