/** Core-local lifecycle rendezvous; not a Plugin API or security grant. */
export const PRODUCT_SESSION_END_EVENT = "cowboy:product-sign-out";
export type ProductSessionEndOutcome = "drained" | "failed" | "pending";

const lifetimes = new WeakMap<EventTarget, AbortController>();

function lifetime(target: EventTarget): AbortController {
  let owner = lifetimes.get(target);
  if (!owner) {
    owner = new AbortController();
    lifetimes.set(target, owner);
  }
  return owner;
}

/** Core page authority lifetime, not a dismissible view or network connection.
 * Late consumers see the same ended signal; a reload creates the new lifetime.
 * Reading it performs no fetch, socket, storage or event-listener registration.
 */
export function productSessionSignal(
  target: EventTarget = globalThis,
): AbortSignal {
  return lifetime(target).signal;
}

export class ProductSessionEndEvent extends Event {
  constructor(readonly waitUntil: (cleanup: Promise<void>) => void) {
    super(PRODUCT_SESSION_END_EVENT);
  }
}

/** Observers seal synchronously and register their real cleanup barriers. A
 * navigation deadline does NOT cancel them or declare a stuck owner disposed.
 * Server logout happens independently; broken storage must not trap sign-out.
 */
export async function announceProductSessionEnd(
  target: EventTarget = globalThis,
  timeoutMs = 1000,
): Promise<ProductSessionEndOutcome> {
  if (!Number.isInteger(timeoutMs) || timeoutMs < 1 || timeoutMs > 5000) {
    throw new RangeError("product cleanup deadline must be 1..5000ms");
  }
  // Fence all borrowed continuations before any event observer or asynchronous
  // navigation cleanup can run. Ending authority never sends remote cleanup.
  lifetime(target).abort();
  const pending: Promise<void>[] = [];
  let accepting = true;
  const event = new ProductSessionEndEvent((cleanup) => {
    if (!accepting) throw new Error("product cleanup admission is closed");
    pending.push(cleanup);
  });
  try {
    target.dispatchEvent(event);
  } finally {
    accepting = false;
  }
  const drained = Promise.allSettled(pending).then((
    outcomes,
  ): ProductSessionEndOutcome =>
    outcomes.some((outcome) => outcome.status === "rejected")
      ? "failed"
      : "drained"
  );
  let timer: ReturnType<typeof setTimeout> | undefined;
  // oxlint-disable-next-line promise/avoid-new
  const deadline = new Promise<ProductSessionEndOutcome>((resolve) => {
    timer = setTimeout(() => resolve("pending"), timeoutMs);
  });
  try {
    return await Promise.race([drained, deadline]);
  } finally {
    clearTimeout(timer);
  }
}
