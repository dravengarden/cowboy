import { useEffect, useMemo, useState } from "react";
import { callActive, fetchCall, fetchCalls, type ManagedCallResult } from "../../managedCalls";
import type { GitReviewEntry } from "../../mobile/review/gitReviewModel";
import { type ReviewDelegation, useDelegations } from "./reviewDelegations";
import { type ReviewFinding, reviewOutcome } from "./reviewFindingsModel";

/** Reviews read at once; older delegations stay recorded but unread. */
const READ_LIMIT = 5;
const POLL_MS = 15_000;
/** A fork that names no matching call this long after the ask never started
 *  the review, or was deleted with its calls: stop waiting for it. */
const START_MS = 10 * 60_000;

interface Observed {
  readonly delegation: ReviewDelegation;
  readonly callId?: string | undefined;
  readonly state: "pending" | "done" | "failed";
  readonly result?: ManagedCallResult | null | undefined;
}

export interface ReviewFindings {
  readonly findings: readonly ReviewFinding[];
  /** Reviews asked and not finished, and finished ones without anchorable findings. */
  readonly pending: number;
  readonly incomplete: number;
}

/**
 * The findings of the reviews Desktop Review delegated for `source`: each
 * fork's calls are read through the fork itself (calls belong to their
 * actual parent), matched by task id, and anchored against the change list
 * as it is now.
 */
export function useReviewFindings(source: string | undefined, queue: readonly GitReviewEntry[]): ReviewFindings {
  const delegations = useDelegations(source);
  const reviews = useMemo(
    () => delegations.filter((delegation) => delegation.kind === "review").slice(-READ_LIMIT),
    [delegations],
  );
  const [observed, setObserved] = useState<readonly Observed[]>([]);
  const [tick, setTick] = useState(0);
  useEffect(() => {
    if (reviews.length === 0) {
      setObserved([]);
      return undefined;
    }
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout> | undefined;
    void Promise.all(reviews.map(async (delegation): Promise<Observed> => {
      try {
        const call = (await fetchCalls(delegation.fork, controller.signal))
          .find((item) => item.labels?.review_task === delegation.task);
        if (!call && Date.now() - delegation.atMs > START_MS) return { delegation, state: "failed" };
        if (!call || callActive(call.state)) return { delegation, callId: call?.call_id, state: "pending" };
        if (!call.has_result) return { delegation, callId: call.call_id, state: "failed" };
        const detail = await fetchCall(delegation.fork, call.call_id, controller.signal);
        return { delegation, callId: call.call_id, state: "done", result: detail.result };
      } catch {
        return { delegation, state: "pending" };
      }
    })).then((next) => {
      if (controller.signal.aborted) return;
      setObserved(next);
      if (next.some((item) => item.state === "pending")) {
        timer = globalThis.setTimeout(() => setTick((value) => value + 1), POLL_MS);
      }
    });
    return () => {
      controller.abort();
      globalThis.clearTimeout(timer);
    };
  }, [reviews, tick]);
  return useMemo((): ReviewFindings => {
    const fingerprints = new Map(queue.map((entry) => [entry.change.path, entry.change.fingerprint]));
    let incomplete = 0;
    const findings = observed.flatMap((item) => {
      if (item.state !== "done" || !item.callId) {
        if (item.state === "failed") incomplete++;
        return [];
      }
      const outcome = reviewOutcome(item.callId, item.result ?? null, item.delegation.files, (path) => fingerprints.get(path));
      if (!outcome.complete) incomplete++;
      return outcome.findings;
    });
    return { findings, pending: observed.filter((item) => item.state === "pending").length, incomplete };
  }, [observed, queue]);
}
