import { CodeApiError, isTransientCodeApiStatus } from "./codeApi";

// A Git review surface reloads when the worktree revision moves. A surface
// that failed while its Machine was reconnecting has no revision to wait for:
// the fetch failed before any revision was read, and an unchanged worktree
// never produces the next one. Recover on a short backoff instead, then settle
// at the cadence the manifest poll already uses.
const REVIEW_RETRY_DELAYS_MS = [800, 2_000, 5_000, 15_000, 30_000] as const;

export function reviewRetryDelayMs(attempt: number): number {
  const index = Math.min(
    Math.max(Math.trunc(attempt), 0),
    REVIEW_RETRY_DELAYS_MS.length - 1,
  );
  return REVIEW_RETRY_DELAYS_MS[index] ?? REVIEW_RETRY_DELAYS_MS[0];
}

/**
 * A Machine that is away answers 502 and comes back seconds later; an offline
 * phone rejects the fetch outright. Both are worth another attempt. A refused
 * or missing resource is a durable answer that only the user can act on.
 */
export function isRecoverableReviewFailure(reason: unknown): boolean {
  if (reason instanceof DOMException && reason.name === "AbortError") {
    return false;
  }
  if (reason instanceof CodeApiError) {
    return isTransientCodeApiStatus(reason.status);
  }
  return true;
}
