import { useCallback, useEffect, useRef, useState } from "react";
import { send } from "./store";

// Answering a tool permission is not instant: the answer crosses the socket,
// reaches the session's worker (possibly on another machine) and only then
// comes back as `permission_resolved`, which removes the request. Until that
// echo the surface must show that the answer was taken, or a press looks like
// it did nothing and gets repeated.
//
//   unsent  the socket was closed; the answer is retried until it leaves
//   sent    the answer left; waiting for the session to confirm
//   stalled no confirmation in time; the choice is offered again

/** How long a sent answer waits for its echo before it is offered again. */
export const PERMISSION_CONFIRM_TIMEOUT_MS = 10_000;
/** How often an answer that could not leave is retried. */
export const PERMISSION_RESEND_INTERVAL_MS = 1_000;

export type PermissionResponsePhase = "unsent" | "sent" | "stalled";

interface Attempt {
  requestId: string;
  optionId: string;
  phase: PermissionResponsePhase;
}

export interface PermissionResponse {
  /** The option answered for this request, if any. */
  optionId: string | null;
  phase: PermissionResponsePhase | null;
  /** True while a press must not answer again. */
  busy: boolean;
  /** What to tell the person when the answer is not confirmed. */
  notice: string | null;
  respond: (optionId: string) => void;
}

export function permissionResponseNotice(
  phase: PermissionResponsePhase | null,
): string | null {
  if (phase === "unsent") {
    return "Not connected. Your answer is sent as soon as the connection returns.";
  }
  if (phase === "stalled") {
    return "The session has not confirmed your answer yet. You can answer again.";
  }
  return null;
}

export function usePermissionResponse(
  sessionId: string,
  requestId: string,
): PermissionResponse {
  const [attempt, setAttempt] = useState<Attempt | null>(null);
  // Keyed by request: the next request starts unanswered without a reset.
  const current = attempt?.requestId === requestId ? attempt : null;
  const phase = current?.phase ?? null;
  const optionId = current?.optionId ?? null;

  const deliver = useCallback((option: string): boolean =>
    send({
      type: "permission",
      session_id: sessionId,
      request_id: requestId,
      option_id: option,
    }), [requestId, sessionId]);

  const latest = useRef(attempt);
  latest.current = attempt;
  const respond = useCallback((option: string): void => {
    const active = latest.current?.requestId === requestId
      ? latest.current
      : null;
    // One answer per request; only a stalled one may be given again.
    if (active && active.phase !== "stalled") return;
    const next: Attempt = {
      requestId,
      optionId: option,
      phase: deliver(option) ? "sent" : "unsent",
    };
    latest.current = next;
    setAttempt(next);
  }, [deliver, requestId]);

  useEffect(() => {
    if (phase === null || phase === "stalled" || optionId === null) {
      return undefined;
    }
    const settle = (next: PermissionResponsePhase): void =>
      setAttempt((previous) =>
        previous?.requestId === requestId && previous.optionId === optionId
          ? { ...previous, phase: next }
          : previous
      );
    if (phase === "unsent") {
      const timer = globalThis.setInterval(() => {
        if (deliver(optionId)) settle("sent");
      }, PERMISSION_RESEND_INTERVAL_MS);
      return () => globalThis.clearInterval(timer);
    }
    const timer = globalThis.setTimeout(
      () => settle("stalled"),
      PERMISSION_CONFIRM_TIMEOUT_MS,
    );
    return () => globalThis.clearTimeout(timer);
  }, [deliver, optionId, phase, requestId]);

  return {
    optionId,
    phase,
    busy: phase === "sent" || phase === "unsent",
    notice: permissionResponseNotice(phase),
    respond,
  };
}
