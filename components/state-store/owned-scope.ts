/** Process-local resource ownership, not authority or durable compensation.
 * No effect is acquired by constructing a scope. Disposal seals entry points
 * synchronously, drains admitted tasks, then releases resources in reverse
 * registration order. Register providers before consumers.
 */
export type ScopePhase = "active" | "draining" | "disposed" | "needs_reconcile";

export interface ScopeSnapshot {
  readonly phase: ScopePhase;
  readonly tasks: number;
  readonly resources: number;
  readonly failures: number;
}

export class ScopeClosedError extends Error {
  constructor() {
    super("resource scope is closed");
    this.name = "ScopeClosedError";
  }
}

export interface OwnedResourceScope {
  readonly active: boolean;
  /** Cooperative cancellation only; the task must still settle to be drained. */
  readonly signal: AbortSignal;
  snapshot(): ScopeSnapshot;
  assertActive(): void;
  /** Register an already-owned cleanup while active. Never acquire externally
   * and then register after an await: register the resource holder FIRST.
   * The returned early release is idempotent, including after failure.
   * Finalizers must not await this scope's own dispose() (a dependency cycle).
   */
  defer(cleanup: () => void | Promise<void>): () => Promise<void>;
  /** Admit a bounded task before invoking it. Its caller owns its result/error;
   * disposal waits for settlement but does not undo its effects or interpret a
   * task failure as a cleanup failure. Fence state publication after each await.
   */
  run<T>(task: () => Promise<T>): Promise<T>;
  /** Fence observations only, never silently discard submitted operations. */
  guard<A extends unknown[]>(
    callback: (...args: A) => void,
  ): (...args: A) => void;
  /** Stable promise, no automatic retry. A failed finalizer remains counted and
   * rejects with AggregateError; other finalizers are still attempted. A stuck
   * task/finalizer stays draining, never a fictitious successful disposal.
   */
  dispose(): Promise<void>;
}

export function createOwnedResourceScope(): OwnedResourceScope {
  const controller = new AbortController();
  const tasks = new Set<Promise<unknown>>();
  const resources = new Set<() => Promise<void>>();
  const failures: unknown[] = [];
  let phase: ScopePhase = "active";
  let disposal: Promise<void> | undefined;
  const assertActive = (): void => {
    if (phase !== "active") throw new ScopeClosedError();
  };
  return {
    get active(): boolean {
      return phase === "active";
    },
    signal: controller.signal,
    snapshot: (): ScopeSnapshot =>
      Object.freeze({
        phase,
        tasks: tasks.size,
        resources: resources.size,
        failures: failures.length,
      }),
    assertActive,
    defer: (cleanup): () => Promise<void> => {
      assertActive();
      let finish: (() => void | Promise<void>) | undefined = cleanup;
      let completion: Promise<void> | undefined;
      const release = (): Promise<void> => {
        // Install the promise BEFORE invoking user code, including a reentrant
        // release/dispose. Retain failed ownership but never invoke it twice.
        completion ??= Promise.resolve().then(() => {
          const action = finish!;
          finish = undefined;
          return action();
        }).then(() => {
          resources.delete(release);
        }, (error: unknown) => {
          failures.push(error);
          throw error;
        });
        return completion;
      };
      resources.add(release);
      return release;
    },
    run: <T>(task: () => Promise<T>): Promise<T> => {
      assertActive();
      // Reserve admission BEFORE invoking external code without postponing the
      // synchronous part of an optimistic mutation to a microtask.
      let resolve!: (value: T | PromiseLike<T>) => void;
      let reject!: (reason: unknown) => void;
      // oxlint-disable-next-line promise/avoid-new
      const result = new Promise<T>((yes, no) => {
        resolve = yes;
        reject = no;
      });
      tasks.add(result);
      void result.then(() => tasks.delete(result), () => tasks.delete(result));
      try {
        resolve(task());
      } catch (error) {
        reject(error);
      }
      return result;
    },
    guard: (callback) => (...args): void => {
      if (phase === "active") callback(...args);
    },
    dispose: (): Promise<void> => {
      if (disposal) return disposal;
      phase = "draining"; // fence BEFORE abort callbacks can reenter
      disposal = Promise.resolve().then(async () => {
        await Promise.allSettled([...tasks]);
        for (const release of [...resources].reverse()) {
          try {
            await release();
          } catch {
            /* keep failure visible; continue releasing other owners */
          }
        }
        phase = failures.length ? "needs_reconcile" : "disposed";
        if (failures.length) {
          throw new AggregateError(failures, "resource cleanup failed");
        }
      });
      controller.abort();
      return disposal;
    },
  };
}
