import { useCallback, useEffect, useState, useSyncExternalStore } from "react";
import { agentLabel } from "./agentTools";

// Agent calls a session's agent is waiting for a person to approve. The
// Controller pushes `call_approval` whenever the waiting set changes and again
// on every resubmission (every few seconds while the agent waits), so a
// reconnecting client recovers the prompt without a read. A prompt that is not
// refreshed within its `ttl_ms` has been abandoned by the agent and hides.
//
// A decision binds to the requests the person was shown (docs/background-
// tasks.md §5): the prompt names them in `request_ids` with a
// `prompt_revision`, and the answer sends exactly that set as `presented`. A
// request that arrives later never inherits an earlier "once"; the Controller
// pushes a refreshed prompt for it instead. Only a prompt on screen can be
// answered: an answer never reads the latest prompt when clicked. A Controller
// older than the binding sends no `request_ids`, and the answer keeps its old
// shape for such a displayed prompt.

export type CallApprovalDecision = "once" | "session" | "decline";

export interface CallApprovalItem {
  /** Stable identity of this waiting request (Controllers with binding). */
  approval_request_id?: string;
  agent: string;
  purpose: string;
  summary: string;
}

export interface CallApproval {
  schema: 1;
  caller: string;
  reason: "calls_disabled" | "policy_denied";
  requests: number;
  agents: string[];
  items: CallApprovalItem[];
  /** Every request this prompt presents (Controllers with binding). */
  request_ids?: string[];
  /** Advances whenever the waiting set changes. */
  prompt_revision?: number;
  ttl_ms: number;
}

/** The body of an answer to `approval`: bound to its presented requests when
 * the Controller supports binding, else the legacy aggregate decision. */
export function callApprovalDecisionBody(
  decision: CallApprovalDecision,
  approval: CallApproval,
): Record<string, unknown> {
  const presented = approval.request_ids;
  if (!Array.isArray(presented) || presented.length === 0) return { decision };
  return {
    decision,
    presented,
    ...(typeof approval.prompt_revision === "number"
      ? { prompt_revision: approval.prompt_revision }
      : {}),
  };
}

interface Entry {
  approval: CallApproval;
  until: number;
}

const entries = new Map<string, Entry>();
const listeners = new Set<() => void>();

function emit(): void {
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return (): void => {
    listeners.delete(listener);
  };
}

/** Fold one pushed prompt (or its removal) into the local view. */
export function receiveCallApproval(
  sessionId: string,
  approval: CallApproval | null,
  now = Date.now(),
): void {
  if (approval === null) {
    if (!entries.delete(sessionId)) return;
  } else {
    entries.set(sessionId, { approval, until: now + approval.ttl_ms });
  }
  emit();
}

export function currentCallApproval(
  sessionId: string,
  now = Date.now(),
): CallApproval | null {
  const entry = entries.get(sessionId);
  return entry && entry.until > now ? entry.approval : null;
}

/** The session's live prompt; it disappears on its own once it expires. */
export function useCallApproval(sessionId: string): CallApproval | null {
  const entry = useSyncExternalStore(
    subscribe,
    () => entries.get(sessionId),
    () => undefined,
  );
  useEffect(() => {
    if (!entry) return;
    const timer = globalThis.setTimeout(() => {
      if (entries.get(sessionId) === entry) {
        entries.delete(sessionId);
        emit();
      }
    }, Math.max(0, entry.until - Date.now()));
    return (): void => globalThis.clearTimeout(timer);
  }, [entry, sessionId]);
  return entry && entry.until > Date.now() ? entry.approval : null;
}

/** "Claude wants to call Codex" / "… to start 3 calls (Codex, Claude)". */
export function callApprovalTitle(approval: CallApproval): string {
  const caller = agentLabel(approval.caller);
  const agents = approval.agents.map(agentLabel);
  if (approval.requests === 1) {
    return `${caller} wants to call ${agents[0] ?? "another agent"}`;
  }
  return `${caller} wants to start ${approval.requests} calls (${
    agents.join(", ")
  })`;
}

export function callApprovalReason(approval: CallApproval): string {
  return approval.reason === "calls_disabled"
    ? "Agent calls are off for this session."
    : `${
      approval.agents.map(agentLabel).join(", ")
    } is not an allowed call target for this session.`;
}

export async function decideCallApproval(
  sessionId: string,
  decision: CallApprovalDecision,
  presented: CallApproval,
): Promise<void> {
  const response = await fetch(
    `/api/sessions/${encodeURIComponent(sessionId)}/calls/approval`,
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(callApprovalDecisionBody(decision, presented)),
    },
  );
  // 409: nothing the person saw still waits (the agent stopped asking or the
  // prompt expired). The Controller pushes the current prompt, if any.
  if (response.status === 409) {
    retireAnswered(sessionId, presented);
    return;
  }
  if (!response.ok) {
    throw new Error(`Could not answer the call request (${response.status})`);
  }
  retireAnswered(sessionId, presented);
}

/** Hide the answered prompt unless a refreshed one (a request that arrived
 * after it was shown) already replaced it. */
function retireAnswered(sessionId: string, answered: CallApproval): void {
  const current = entries.get(sessionId)?.approval;
  if (current === undefined) return;
  if (
    current !== answered &&
    typeof current.prompt_revision === "number" &&
    current.prompt_revision !== answered.prompt_revision
  ) return;
  receiveCallApproval(sessionId, null);
}

export interface CallApprovalAnswer {
  busy: CallApprovalDecision | null;
  failure: string | null;
  answer: (decision: CallApprovalDecision) => void;
}

/** One answer at a time, with the refusal kept for the surface to show. */
export function useCallApprovalAnswer(
  sessionId: string,
  approval: CallApproval,
): CallApprovalAnswer {
  const [busy, setBusy] = useState<CallApprovalDecision | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const answer = useCallback((decision: CallApprovalDecision): void => {
    if (busy) return;
    setBusy(decision);
    setFailure(null);
    decideCallApproval(sessionId, decision, approval).catch((reason: unknown) => {
      setFailure(
        reason instanceof Error ? reason.message : "Could not answer",
      );
    }).finally(() => setBusy(null));
  }, [approval, busy, sessionId]);
  return { busy, failure, answer };
}
