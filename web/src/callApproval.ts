import { useEffect, useSyncExternalStore } from "react";
import { agentLabel } from "./agentTools";

// Agent calls a session's agent is waiting for a person to approve. The
// Controller pushes `call_approval` whenever the waiting set changes and again
// on every resubmission (every few seconds while the agent waits), so a
// reconnecting client recovers the prompt without a read. A prompt that is not
// refreshed within its `ttl_ms` has been abandoned by the agent and hides.

export type CallApprovalDecision = "once" | "session" | "decline";

export interface CallApprovalItem {
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
  ttl_ms: number;
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
): Promise<void> {
  const response = await fetch(
    `/api/sessions/${encodeURIComponent(sessionId)}/calls/approval`,
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ decision }),
    },
  );
  // 409: the agent stopped waiting before the answer arrived.
  if (response.status === 409) {
    receiveCallApproval(sessionId, null);
    return;
  }
  if (!response.ok) {
    throw new Error(`Could not answer the call request (${response.status})`);
  }
  receiveCallApproval(sessionId, null);
}
