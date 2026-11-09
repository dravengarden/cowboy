import { useEffect, useMemo, useRef, useState } from "react";
import { callActive, fetchCall, fetchCalls, type ManagedCallDetail, type ManagedCallResult } from "../../managedCalls";
import { CodeApiError, fetchCodeFileBlob } from "../../review/codeApi";
import type { GitReviewEntry } from "../../review/gitReviewModel";
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
  /** Per reviewed file: its blob in the snapshot and now (`null`: gone),
   *  read while the file had `fingerprint`. */
  readonly blobs?: ReadonlyMap<string, FileBlobs> | undefined;
}

interface FileBlobs {
  readonly snapshot: string;
  readonly current: string | null | undefined;
  readonly fingerprint: string | undefined;
}

/** The reviewed files' blobs now, beside the ones the review captured. A
 *  context or Machine that cannot answer leaves `current` unknown. */
async function readBlobs(
  context: string,
  detail: ManagedCallDetail,
  delegation: ReviewDelegation,
  fingerprintOf: (path: string) => string | undefined,
  signal: AbortSignal,
): Promise<ReadonlyMap<string, FileBlobs>> {
  const captured = new Map((detail.input_files ?? []).map((file) => [file.path, file.blob]));
  const blobs = new Map<string, FileBlobs>();
  for (const { path } of delegation.files) {
    const snapshot = captured.get(path);
    if (!snapshot) continue;
    const fingerprint = fingerprintOf(path);
    const current = await fetchCodeFileBlob(context, path, signal).then(
      (blob) => blob.oid,
      (reason: unknown) => reason instanceof CodeApiError && reason.status === 404 ? null : undefined,
    );
    blobs.set(path, { snapshot, current, fingerprint });
  }
  return blobs;
}

/** Whether the file still is the reviewed blob: unknown when either side is,
 *  and false once the file changed after its blob was read. */
function sameBlob(blobs: FileBlobs | undefined, fingerprint: string | undefined): boolean | undefined {
  if (!blobs || blobs.current === undefined) return undefined;
  return blobs.fingerprint === fingerprint && blobs.current === blobs.snapshot;
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
export function useReviewFindings(
  source: string | undefined,
  queue: readonly GitReviewEntry[],
  /** Review is shown: reads run only then, and showing it again re-checks
   *  settled reviews (a deleted fork takes its findings with it). */
  active: boolean,
): ReviewFindings {
  const delegations = useDelegations(source);
  const reviews = useMemo(
    () => delegations.filter((delegation) => delegation.kind === "review").slice(-READ_LIMIT),
    [delegations],
  );
  const fingerprints = useRef(new Map<string, string | undefined>());
  fingerprints.current = new Map(queue.map((entry) => [entry.change.path, entry.change.fingerprint]));
  const reviewedKey = reviews
    .flatMap((delegation) => delegation.files.map((file) => `${file.path}\0${fingerprints.current.get(file.path) ?? ""}`))
    .join("\n");
  const [observed, setObserved] = useState<readonly Observed[]>([]);
  const [tick, setTick] = useState(0);
  useEffect(() => {
    if (reviews.length === 0) {
      setObserved([]);
      return undefined;
    }
    if (!active) return undefined;
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout> | undefined;
    void Promise.all(reviews.map(async (delegation): Promise<Observed> => {
      try {
        const call = (await fetchCalls(delegation.fork, controller.signal))
          .find((item) => item.labels?.review_task === delegation.task);
        if (!call && Date.now() - delegation.atMs > START_MS) return { delegation, state: "failed" };
        if (!call || callActive(call.state)) return { delegation, callId: call?.call_id, state: "pending" };
        // Only a completed call's result is a review; a failed one may still
        // carry the rejected output.
        if (call.state !== "completed" || !call.has_result) return { delegation, callId: call.call_id, state: "failed" };
        const detail = await fetchCall(delegation.fork, call.call_id, controller.signal);
        const fingerprintOf = (path: string) => fingerprints.current.get(path);
        const blobs = await readBlobs(delegation.context ?? source ?? "", detail, delegation, fingerprintOf, controller.signal);
        return { delegation, callId: call.call_id, state: "done", result: detail.result, blobs };
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
    // A reviewed file's change reads its blob again.
  }, [active, reviews, tick, reviewedKey]);
  return useMemo((): ReviewFindings => {
    const fingerprints = new Map(queue.map((entry) => [entry.change.path, entry.change.fingerprint]));
    let incomplete = 0;
    const findings = observed.flatMap((item) => {
      if (item.state !== "done" || !item.callId) {
        if (item.state === "failed") incomplete++;
        return [];
      }
      const outcome = reviewOutcome(
        item.callId,
        item.result ?? null,
        item.delegation.files,
        (path) => fingerprints.get(path),
        (path) => sameBlob(item.blobs?.get(path), fingerprints.get(path)),
      );
      if (!outcome.complete) incomplete++;
      return outcome.findings;
    });
    return { findings, pending: observed.filter((item) => item.state === "pending").length, incomplete };
  }, [observed, queue]);
}
