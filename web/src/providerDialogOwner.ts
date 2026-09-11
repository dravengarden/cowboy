import { createOwnedResourceScope } from "@cowboy/state-store/scope";

export interface ProviderDialogSnapshot<T, A extends string> {
  readonly value: T | null;
  readonly busy: A | null;
  readonly error: string;
}

function freezeData<T>(value: T): T {
  if (value !== null && typeof value === "object" && !Object.isFrozen(value)) {
    for (const child of Object.values(value)) freezeData(child);
    Object.freeze(value);
  }
  return value;
}

/** Core-only local observation owner. A lease is an object identity, not a
 * request ID or remote authority. Retiring it aborts READ observations only;
 * admitted writes must not use its signal and remain counted until settlement.
 */
export function createProviderDialogOwner<T, A extends string>() {
  const root = createOwnedResourceScope();
  const listeners = new Set<() => void>();
  let snapshot: ProviderDialogSnapshot<T, A> = Object.freeze({
    value: null,
    busy: null,
    error: "",
  });
  let current: ReturnType<typeof createLease> | undefined;
  function publish(next: ProviderDialogSnapshot<T, A>): void {
    snapshot = Object.freeze(next);
    // New listeners belong to the next notification; observers cannot prevent
    // an already admitted request or interrupt teardown.
    // oxlint-disable-next-line unicorn/no-useless-spread
    for (const listener of [...listeners]) {
      if (!listeners.has(listener)) continue;
      try {
        listener();
      } catch { /* Rendering is not execution authority. */ }
    }
  }
  function createLease(value: T) {
    const scope = createOwnedResourceScope();
    const release = root.defer(() => scope.dispose());
    const lease = {
      get active(): boolean {
        return root.active && scope.active && current === lease;
      },
      signal: scope.signal,
      value: () => value,
      update(update: (previous: T) => T): void {
        if (!lease.active) return;
        value = freezeData(update(value));
        publish({ ...snapshot, value });
      },
      error(detail: string): void {
        if (lease.active) publish({ ...snapshot, error: detail });
      },
      observe: scope.run,
      defer: scope.defer,
      run(
        action: A,
        fallback: string,
        work: () => Promise<void>,
      ): Promise<void> {
        if (!lease.active || snapshot.busy !== null) return Promise.resolve();
        return scope.run(async () => {
          publish({ value, busy: action, error: "" });
          try {
            await work();
          } catch (cause) {
            lease.error(cause instanceof Error ? cause.message : fallback);
            throw cause;
          } finally {
            if (lease.active) publish({ ...snapshot, busy: null });
          }
        });
      },
      retire(): void {
        // Seal synchronously, BEFORE the asynchronous root finalizer runs.
        void scope.dispose().catch(() => {});
        void release().catch(() => {}); // root retains any cleanup failure
      },
    };
    return lease;
  }
  return {
    snapshot: () => snapshot,
    lifecycle: root.snapshot,
    subscribe(listener: () => void): () => void {
      if (root.active) listeners.add(listener);
      return () => listeners.delete(listener);
    },
    current: () => current?.active ? current : undefined,
    open(value: T) {
      if (!root.active) return undefined;
      // Retired writes can honestly remain draining. Do not accumulate an
      // unbounded number of new incarnations behind a stuck transport.
      if (root.snapshot().resources >= 16) {
        publish({
          ...snapshot,
          error:
            "Previous Provider requests are still draining. Try again later.",
        });
        return undefined;
      }
      const previous = current;
      current = undefined;
      previous?.retire();
      current = createLease(freezeData(value));
      const admitted = current;
      publish({ value: admitted.value(), busy: null, error: "" });
      return admitted;
    },
    close(lease = current): void {
      if (!lease || lease !== current) return;
      current = undefined;
      lease.retire();
      publish({ value: null, busy: null, error: "" });
    },
    dispose(): Promise<void> {
      listeners.clear();
      const previous = current;
      current = undefined;
      previous?.retire();
      snapshot = Object.freeze({ value: null, busy: null, error: "" });
      return root.dispose();
    },
  };
}

export async function expectProviderResponse(
  response: Response,
  fallback: string,
): Promise<void> {
  if (response.ok) return;
  const body = (await response.text()).trim();
  let parsed: { detail?: unknown; error?: unknown } | undefined;
  try {
    parsed = JSON.parse(body) as typeof parsed;
  } catch { /* Preserve a non-JSON server response below. */ }
  if (typeof parsed?.detail === "string" && parsed.detail.trim()) {
    throw new Error(parsed.detail.trim());
  }
  if (typeof parsed?.error === "string" && parsed.error.trim()) {
    throw new Error(parsed.error.trim());
  }
  throw new Error(body || fallback);
}
