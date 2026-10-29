import { requestUsageRefresh } from "./store";

/** Concurrent callers share the in-flight run; the next call starts anew. */
export function singleFlight(run: () => Promise<void>): () => Promise<void> {
  let pending: Promise<void> | undefined;
  return () => {
    pending ??= run().finally(() => {
      pending = undefined;
    });
    return pending;
  };
}

/** The one global usage refresh. The Controller refreshes every Service
 *  account and, serially, this owner's named accounts; results reach every
 *  surface through the shared usage stores and their broadcasts. Concurrent
 *  callers share one request. */
export const refreshAllUsage = singleFlight(() => requestUsageRefresh());
