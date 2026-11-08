import { type RefObject, useCallback, useEffect, useRef } from "react";
import {
  isRecoverableReviewFailure,
  reviewRetryDelayMs,
} from "./reviewRecovery";

export interface ReviewRecovery {
  /** A load failed: retry it on the backoff unless the answer was durable. */
  armRetry: (reason: unknown) => void;
  /** A load started: drop a scheduled retry, keep the attempt count. */
  cancelRetry: () => void;
  /** A load succeeded: the next failure starts from the shortest delay. */
  settleRetry: () => void;
}

/**
 * Keep a Git review surface trying after a transient failure instead of
 * leaving an error that only a tap can clear. A hidden page schedules nothing
 * and recovers when it is looked at again, so a phone in a pocket never polls
 * a Machine it cannot show.
 */
export function useReviewRecovery(
  reload: RefObject<() => void>,
): ReviewRecovery {
  const timer = useRef<number | undefined>(undefined);
  const attempt = useRef(0);
  const pending = useRef(false);

  const clearTimer = useCallback((): void => {
    if (timer.current === undefined) return;
    globalThis.clearTimeout(timer.current);
    timer.current = undefined;
  }, []);

  const cancelRetry = useCallback((): void => {
    clearTimer();
    pending.current = false;
  }, [clearTimer]);

  const settleRetry = useCallback((): void => {
    clearTimer();
    pending.current = false;
    attempt.current = 0;
  }, [clearTimer]);

  const armRetry = useCallback((reason: unknown): void => {
    if (!isRecoverableReviewFailure(reason)) {
      settleRetry();
      return;
    }
    clearTimer();
    pending.current = true;
    if (globalThis.document?.visibilityState === "hidden") return;
    const delay = reviewRetryDelayMs(attempt.current);
    attempt.current += 1;
    timer.current = globalThis.setTimeout(() => {
      timer.current = undefined;
      pending.current = false;
      reload.current();
    }, delay);
  }, [clearTimer, reload, settleRetry]);

  useEffect(() => {
    // Returning to the app, or to the network, is better evidence than any
    // remaining wait: try at once and start the backoff over.
    const wake = (): void => {
      if (!pending.current) return;
      if (globalThis.document?.visibilityState === "hidden") return;
      clearTimer();
      pending.current = false;
      attempt.current = 0;
      reload.current();
    };
    globalThis.document?.addEventListener("visibilitychange", wake);
    globalThis.addEventListener("online", wake);
    return () => {
      globalThis.document?.removeEventListener("visibilitychange", wake);
      globalThis.removeEventListener("online", wake);
      clearTimer();
    };
  }, [clearTimer, reload]);

  return { armRetry, cancelRetry, settleRetry };
}
