import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import type { CodeDiffScope } from "./codeApi.ts";
import {
  projectedDiffPoint,
  projectReviewDiff,
  type ReviewDiffProjection,
} from "./ownedDiffProjection.ts";
import {
  productReviewDiffSource,
  readReviewDiffSource,
  type ReviewDiffSource,
} from "./ownedDiffSource.ts";

export type ReviewDiffStatus =
  | "historical"
  | "incomplete"
  | "checking"
  | "mismatch"
  | "unavailable"
  | "matched";

/** One patch observation owns its complete-file projection. A replacement
 * patch ends this observer even if both patches happen to map to equal text.
 */
export function useOwnedReviewDiff(
  sessionId: string,
  path: string,
  enabled: boolean,
  scope: CodeDiffScope,
  completePatch: string | undefined,
  source: ReviewDiffSource = productReviewDiffSource,
) {
  const [attempt, setAttempt] = useState(0);
  const key = useMemo(
    () => ({ sessionId, path, scope, completePatch, enabled, source, attempt }),
    [sessionId, path, scope, completePatch, enabled, source, attempt],
  );
  const [observation, setObservation] = useState<{
    key: typeof key;
    status: ReviewDiffStatus;
    projection?: ReviewDiffProjection;
    signal: AbortSignal;
  }>();
  const observer = useRef<AbortController | undefined>(undefined);
  useLayoutEffect(() => () => observer.current?.abort(), [key]);
  useEffect(() => {
    const { enabled, completePatch, scope, source, sessionId, path } = key;
    if (!enabled || completePatch === undefined || scope === "staged") return;
    const controller = new AbortController();
    observer.current = controller;
    const signal = AbortSignal.any([controller.signal, source.signal]);
    const ended = () => setObservation({ key, status: "unavailable", signal });
    source.signal.addEventListener("abort", ended, { once: true });
    void readReviewDiffSource(source, { sessionId, path }, signal).then(
      (text) => {
        if (signal.aborted) return;
        const projection = projectReviewDiff(completePatch, text);
        setObservation({
          key,
          status: projection ? "matched" : "mismatch",
          ...(projection ? { projection } : {}),
          signal,
        });
      },
    ).catch(() => {
      if (!signal.aborted) {
        setObservation({ key, status: "unavailable", signal });
      }
    });
    return () => {
      source.signal.removeEventListener("abort", ended);
      controller.abort();
    };
  }, [key]);
  const matching = observation?.key === key && !observation.signal.aborted
    ? observation
    : undefined;
  const projection = matching?.projection;
  const status: ReviewDiffStatus = scope === "staged"
    ? "historical"
    : !enabled || completePatch === undefined
    ? "incomplete"
    : source.signal.aborted
    ? "unavailable"
    : matching?.status ?? "checking";
  const point = useCallback(
    (row: number, column: number) =>
      projection && !matching?.signal.aborted
        ? projectedDiffPoint(projection, row, column)
        : null,
    [projection, matching],
  );
  return {
    status,
    projection,
    point,
    check: () => {
      observer.current?.abort();
      setAttempt((value) => value + 1);
    },
  };
}
