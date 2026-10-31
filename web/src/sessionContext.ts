import { useCallback, useEffect, useState } from "react";
import { PREFERENCES_CHANGED } from "./aboutPreferences";
import { productSessionSignal } from "./productSessionEnd";
import { requestPickSession } from "./sessionPickRequest";
import { newUuid } from "./uuid";
import type { SessionMeta } from "./protocol";

export interface NextAgent {
  provider: string;
  provider_account_id: string;
  machine_id: string;
  runtime_machine_id: string | null;
  cwd: string;
  preset: string | null;
  inherit_settings?: boolean;
}
export interface ContextDelivery {
  operation: string;
  session_id: string;
  state: "preparing" | "accepted" | "failed" | "unknown";
  error?: string | null;
  session?: SessionMeta;
}
export interface SessionContext {
  revision: number;
  applied_revision: number;
  sources: string[];
  next_agent: NextAgent | null;
  source_sessions: {
    id: string;
    available: boolean;
    title?: string;
    provider?: string;
    provider_account_id?: string;
  }[];
  /** Every session this conversation has read, surviving later edits. */
  links?: {
    id: string;
    available: boolean;
    title?: string;
    provider?: string;
    provider_account_id?: string;
    before?: number | null;
  }[];
  delivery: ContextDelivery | null;
  max_sources: number;
}

/** Sources the next message will read. Once sent they are links instead. */
export function pendingSources(context: SessionContext): string[] {
  return context.revision === context.applied_revision ? [] : context.sources;
}

const reads = new Map<string, Promise<SessionContext>>();
const configured = new Set<string>();
const readRevisions = new Map<string, number>();
let principalEpoch = 0;
function invalidateRead(id: string) {
  readRevisions.set(id, (readRevisions.get(id) ?? 0) + 1);
  reads.delete(id);
}
globalThis.addEventListener?.(PREFERENCES_CHANGED, (event) => {
  const scopes =
    (event as CustomEvent<Record<string, number> | undefined>).detail;
  for (const key of Object.keys(scopes ?? {})) {
    if (key.startsWith("session_context:")) {
      const id = key.slice("session_context:".length);
      configured.add(id);
      invalidateRead(id);
    }
  }
  if (!scopes) { for (const id of reads.keys()) invalidateRead(id); }
});
export const CONTEXT_CHANGED = "cowboy:session-context-changed";

class ContextHttpError extends Error {}
async function json<T>(url: string, init?: RequestInit): Promise<T> {
  const response = await fetch(url, {
    cache: "no-store",
    ...init,
    signal: productSessionSignal(),
  });
  const body = await response.text();
  let value: unknown;
  try {
    value = body ? JSON.parse(body) : null;
  } catch {
    if (response.ok) {
      throw new Error(
        "The server reply was unreadable. Check context before retrying.",
      );
    }
  }
  if (!response.ok) {
    const error = (value as { error?: unknown })?.error;
    throw new ContextHttpError(
      typeof error === "string"
        ? error
        : response.status === 413
        ? "This message is too large. Reduce its attachments and try again."
        : `Could not update session context (HTTP ${response.status})`,
    );
  }
  return value as T;
}

export function loadSessionContext(id: string): Promise<SessionContext> {
  const existing = reads.get(id);
  if (existing) return existing;
  const revision = readRevisions.get(id) ?? 0;
  const epoch = principalEpoch;
  const read: Promise<SessionContext> = json<SessionContext>(
    `/api/sessions/${encodeURIComponent(id)}/context`,
  )
    .then((context) => {
      if (epoch !== principalEpoch) throw new Error("Account changed");
      // An announcement can arrive while a GET still holds an older snapshot.
      // Every waiter must see the fresh read, never overwrite it with that reply.
      if (revision !== (readRevisions.get(id) ?? 0)) {
        return loadSessionContext(id);
      }
      if (context.revision > 0) configured.add(id);
      return context;
    })
    .finally(() => {
      if (reads.get(id) === read) reads.delete(id);
    });
  reads.set(id, read);
  return read;
}

/** Whether the Service has announced context for this session. Ordinary
 * sessions never fetch context; consumers re-check on CONTEXT_CHANGED. */
export function contextConfigured(id: string): boolean {
  return configured.has(id) || sends.has(id);
}

export function announceContext(id: string): void {
  invalidateRead(id);
  globalThis.dispatchEvent?.(new CustomEvent(CONTEXT_CHANGED, { detail: id }));
}

/** Back from a staged new agent: other selected sources stay linked. */
export function withoutAgentSwitch(
  context: Pick<SessionContext, "sources">,
  sessionId: string,
): Pick<SessionContext, "sources" | "next_agent"> {
  return {
    sources: context.sources.filter((id) => id !== sessionId),
    next_agent: null,
  };
}

export async function saveSessionContext(
  id: string,
  edit: Pick<SessionContext, "revision" | "sources" | "next_agent"> & {
    resolve_operation?: string;
  },
): Promise<SessionContext> {
  const saved = await json<SessionContext>(
    `/api/sessions/${encodeURIComponent(id)}/context`,
    {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(edit),
    },
  );
  announceContext(id);
  configured.add(id);
  sends.delete(id);
  return saved;
}

export function useSessionContext(id: string, active = true) {
  const [loaded, setLoaded] = useState<{ id: string; data: SessionContext }>();
  const data = loaded?.id === id ? loaded.data : undefined;
  const [error, setError] = useState("");
  const reload = useCallback(async () => {
    try {
      const next = await loadSessionContext(id);
      setLoaded({ id, data: next });
      setError("");
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : "Could not load context",
      );
    }
  }, [id]);
  useEffect(() => {
    if (!active) return;
    let alive = true;
    setLoaded(undefined);
    const refresh = () => {
      void loadSessionContext(id).then((next) => {
        if (alive) {
          setLoaded({ id, data: next });
          setError("");
        }
      })
        .catch((cause: unknown) => {
          if (alive) {
            setError(
              cause instanceof Error ? cause.message : "Could not load context",
            );
          }
        });
    };
    const preference = (event: Event) => {
      const scopes =
        (event as CustomEvent<Record<string, number> | undefined>).detail;
      if (!scopes || Object.hasOwn(scopes, `session_context:${id}`)) refresh();
    };
    const local = (event: Event) => {
      if ((event as CustomEvent<string>).detail === id) refresh();
    };
    refresh();
    window.addEventListener(PREFERENCES_CHANGED, preference);
    window.addEventListener(CONTEXT_CHANGED, local);
    window.addEventListener("focus", refresh);
    return () => {
      alive = false;
      window.removeEventListener(PREFERENCES_CHANGED, preference);
      window.removeEventListener(CONTEXT_CHANGED, local);
      window.removeEventListener("focus", refresh);
    };
  }, [id, active]);
  return { data, error, reload };
}

const sends = new Map<
  string,
  { operation: string; revision: number; text: string; content: unknown[] }
>();
const observedHandoffs = new Set<string>();
export class ContextNavigationNotice extends Error {
  constructor(readonly sessionId: string) {
    super(
      "A previous message continued in the destination now open. Check it before resending. You can return here to keep using the original agent.",
    );
  }
}
export function openContextDestination(receipt: ContextDelivery): void {
  observedHandoffs.add(receipt.operation);
  if (typeof globalThis.dispatchEvent === "function") {
    requestPickSession(receipt.session_id, receipt.session);
  }
}
globalThis.addEventListener?.("cowboy:product-sign-out", () => {
  principalEpoch++;
  sends.clear();
  reads.clear();
  readRevisions.clear();
  configured.clear();
  observedHandoffs.clear();
});

/** A read of Service state fences another tab's staged selection. No Provider
 * query or model call is involved. Uncertain sends retain their operation id. */
export async function requireCurrentAgentContext(id: string): Promise<void> {
  if (!configured.has(id) && !sends.has(id)) return;
  const context = await loadSessionContext(id);
  if (
    context.revision !== context.applied_revision || sends.has(id) ||
    context.delivery?.state === "preparing" ||
    context.delivery?.state === "unknown"
  ) {
    throw new Error(
      "Context changes apply on the next turn. Choose next turn, or cancel the pending context changes.",
    );
  }
}

export async function sendWithContext(
  id: string,
  text: string,
  content: unknown[],
): Promise<boolean> {
  // Ordinary sessions retain their offline outbox and zero extra fetches. The
  // owner-scoped preferences stream announces links across tabs and reconnects;
  // the Service also rejects stale direct sends at the dispatch boundary.
  if (!configured.has(id) && !sends.has(id)) return false;
  const context = await loadSessionContext(id);
  let request = sends.get(id);
  if (
    !request && context.revision === context.applied_revision &&
    context.delivery?.state === "accepted" &&
    context.delivery.session_id !== id &&
    !observedHandoffs.has(context.delivery.operation)
  ) {
    openContextDestination(context.delivery);
    throw new ContextNavigationNotice(context.delivery.session_id);
  }
  if (!request && context.revision === context.applied_revision) return false;
  if (
    request &&
    (request.text !== text ||
      JSON.stringify(request.content) !== JSON.stringify(content))
  ) {
    throw new Error(
      "The previous send is unresolved. Check its destination before submitting another message.",
    );
  }
  request ??= {
    operation: newUuid(),
    revision: context.revision,
    text,
    content,
  };
  sends.set(id, request);
  let receipt: ContextDelivery;
  try {
    receipt = await json<ContextDelivery>(
      `/api/sessions/${encodeURIComponent(id)}/context/send`,
      {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(request),
      },
    );
  } catch (cause) {
    // A definite HTTP refusal can precede admission. A transport failure is
    // ambiguous even if a racing GET has not observed the intent yet.
    if (cause instanceof ContextHttpError) {
      const current = await loadSessionContext(id).catch(() => undefined);
      if (current && current.delivery?.operation !== request.operation) {
        sends.delete(id);
      }
    }
    throw cause;
  }
  announceContext(id);
  if (receipt.state !== "accepted") {
    if (receipt.state === "failed") sends.delete(id);
    throw new Error(
      receipt.error ||
        (receipt.state === "failed"
          ? "This message was not submitted. Check the selected agent and try again."
          : "Send is still being checked. Refresh context to inspect its destination; do not send a duplicate."),
    );
  }
  sends.delete(id);
  if (receipt.session_id !== id) {
    openContextDestination(receipt);
  }
  return true;
}
