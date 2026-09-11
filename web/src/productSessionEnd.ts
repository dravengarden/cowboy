/** Core-local lifecycle rendezvous; not a Plugin API or security grant. */
export const PRODUCT_SESSION_END_EVENT = "cowboy:product-sign-out";
export type ProductSessionEndOutcome = "drained" | "failed" | "pending";

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
