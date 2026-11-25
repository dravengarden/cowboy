import { useCallback, useEffect, useState } from "react";
import { PREFERENCES_CHANGED } from "./aboutPreferences";
import { productSessionSignal } from "./productSessionEnd";
import { requestPickSession } from "./sessionPickRequest";
import { newUuid } from "./uuid";
import type { ContentBlock, SessionMeta } from "./protocol";

export interface NextAgent {
  provider: string;
  provider_account_id: string;
  machine_id: string;
  runtime_machine_id: string | null;
  cwd: string;
  preset: string | null;
  inherit_settings?: boolean;
  /** Explicit options for the new agent, over inherited and preset values. */
  config?: Record<string, string | boolean>;
}
export interface ContextDelivery {
  operation: string;
  session_id: string;
  state: "preparing" | "accepted" | "failed" | "unknown" | "cancelled";
  error?: string | null;
  session?: SessionMeta;
  source_trashed?: boolean;
  /** Why the handoff left its source out of Trash. */
  source_kept?: string;
}
export interface SessionContext {
  revision: number;
  /** The user kept working here after this session's handoff. */
  handoff_resumed?: boolean;
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
  pending_links?: string[];
  delivery: ContextDelivery | null;
  /** A submitted prompt still carries this session's context. While the
   *  session is busy the server refuses a new agent. */
  injection_pending?: boolean;
  max_sources: number;
}

/** Sessions the next prompt will read. Once it carries them they are links. */
export function pendingSources(context: SessionContext): string[] {
  return context.pending_links ?? [];
}

export function pendingAgentSwitch(context: SessionContext | undefined): SessionContext | undefined {
  return context?.next_agent && context.revision !== context.applied_revision ? context : undefined;
}

export function isCurrentContextReady(context: SessionContext | undefined): boolean {
  if (context === undefined) return true;
  return (
    context.revision === context.applied_revision &&
    context.delivery?.state !== "preparing" &&
    context.delivery?.state !== "unknown"
  );
}

export function stagedNextAgent(context: SessionContext): boolean {
  return pendingAgentSwitch(context) !== undefined &&
    context.delivery?.state !== "preparing" &&
    context.delivery?.state !== "unknown";
}

const DISPLAY_CACHE_LIMIT = 64;
const reads = new Map<string, Promise<SessionContext>>();
const cachedContexts = new Map<string, { revision: number; data: SessionContext }>();
const announcedRevisions = new Map<string, number>();
const loadedAnnouncedRevisions = new Map<string, number>();
const configured = new Set<string>();
const readRevisions = new Map<string, number>();
let principalEpoch = 0;

function trimContextForDisplayCache(context: SessionContext): SessionContext {
  if (!context.delivery || !("returned" in context.delivery)) return context;
  const {
    operation,
    session_id,
    state,
    error,
    session,
    source_trashed,
    source_kept,
  } = context.delivery;
  return {
    ...context,
    delivery: {
      operation,
      session_id,
      state,
      ...(error !== undefined ? { error } : {}),
      ...(session !== undefined ? { session } : {}),
      ...(source_trashed !== undefined ? { source_trashed } : {}),
      ...(source_kept !== undefined ? { source_kept } : {}),
    },
  };
}

function storeDisplayCache(id: string, revision: number, context: SessionContext): void {
  cachedContexts.delete(id);
  cachedContexts.set(id, { revision, data: trimContextForDisplayCache(context) });
  while (cachedContexts.size > DISPLAY_CACHE_LIMIT) {
    const oldest = cachedContexts.keys().next().value;
    if (oldest === undefined) break;
    cachedContexts.delete(oldest);
  }
}

function invalidateRead(id: string) {
  readRevisions.set(id, (readRevisions.get(id) ?? 0) + 1);
  reads.delete(id);
  cachedContexts.delete(id);
  loadedAnnouncedRevisions.delete(id);
}

/** Drop cached context state for a deleted session so attachments and metadata
 *  do not linger in memory across a mobile session. */
export function evictSessionContext(id: string): void {
  invalidateRead(id);
  announcedRevisions.delete(id);
  configured.delete(id);
  readRevisions.delete(id);
  sends.delete(id);
}

/** Invalidate cached context views whose linked or source sessions reference
 *  `targetSessionId` (e.g. when a linked session's availability or closing
 *  state changes without bumping the referencing session's context revision). */
export function invalidateReferencingContexts(targetSessionId: string): void {
  const affected: string[] = [];
  for (const [id, entry] of cachedContexts) {
    const referencesTarget =
      entry.data.links?.some((link) => link.id === targetSessionId) === true ||
      entry.data.source_sessions?.some((source) => source.id === targetSessionId) === true;
    if (referencesTarget) affected.push(id);
  }
  for (const id of affected) announceContext(id);
}

globalThis.addEventListener?.(PREFERENCES_CHANGED, (event) => {
  const scopes =
    (event as CustomEvent<Record<string, number> | undefined>).detail;
  if (!scopes) {
    announcedRevisions.clear();
    loadedAnnouncedRevisions.clear();
    for (const id of new Set([...reads.keys(), ...cachedContexts.keys(), ...configured])) {
      invalidateRead(id);
    }
    globalThis.dispatchEvent?.(new CustomEvent(CONTEXT_CHANGED, { detail: null }));
    return;
  }
  for (const [key, revision] of Object.entries(scopes)) {
    if (!key.startsWith("session_context:")) continue;
    const id = key.slice("session_context:".length);
    configured.add(id);
    const previous = announcedRevisions.get(id);
    announcedRevisions.set(id, revision);
    // Skip only when this exact revision already completed a successful load
    // or is currently in flight; a prior failed read must retry on reconnect.
    if (
      previous === revision &&
      (loadedAnnouncedRevisions.get(id) === revision || reads.has(id))
    ) {
      continue;
    }
    invalidateRead(id);
    globalThis.dispatchEvent?.(new CustomEvent(CONTEXT_CHANGED, { detail: id }));
  }
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

export interface CancelledHandoff {
  source_session_id: string;
  text: string;
  content: ContentBlock[];
  session?: SessionMeta | null;
  note?: string;
}

/** Take a continuation back while its first message is still queued. The
 *  Service restores the original and moves this destination to Trash. */
export async function cancelHandoff(
  id: string,
  operation: string,
): Promise<CancelledHandoff> {
  const cancelled = await json<CancelledHandoff>(
    `/api/sessions/${encodeURIComponent(id)}/context/cancel`,
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ operation }),
    },
  );
  // A send whose reply was lost is resolved now: it was taken back.
  sends.delete(cancelled.source_session_id);
  announceContext(id);
  announceContext(cancelled.source_session_id);
  return cancelled;
}

export function loadSessionContext(id: string): Promise<SessionContext> {
  const existing = reads.get(id);
  if (existing) return existing;
  const revision = readRevisions.get(id) ?? 0;
  const announced = announcedRevisions.get(id);
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
      if (announced !== undefined && announcedRevisions.get(id) === announced) {
        loadedAnnouncedRevisions.set(id, announced);
        storeDisplayCache(id, revision, context);
      }
      return context;
    })
    .finally(() => {
      if (reads.get(id) === read) reads.delete(id);
    });
  reads.set(id, read);
  return read;
}

function readDisplaySessionContext(id: string): Promise<SessionContext> {
  // Only reuse the display cache when the Controller announces revisions for
  // this session; ownerless sessions (product auth disabled) revalidate on mount.
  if (announcedRevisions.has(id)) {
    const cached = cachedContexts.get(id);
    const revision = readRevisions.get(id) ?? 0;
    if (cached !== undefined && cached.revision === revision) {
      cachedContexts.delete(id);
      cachedContexts.set(id, cached);
      return Promise.resolve(cached.data);
    }
  }
  return loadSessionContext(id);
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

const linkEdits = new Map<string, Promise<void>>();

/** A prompt sent right after linking must not overtake that link. */
export function trackLinkEdit(id: string, edit: Promise<void>): void {
  const tracked = edit.finally(() => {
    if (linkEdits.get(id) === tracked) linkEdits.delete(id);
  });
  linkEdits.set(id, tracked);
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

/** Keep working in a handed-off session beside its continuation. */
export async function continueHere(id: string): Promise<void> {
  await json<unknown>(`/api/sessions/${encodeURIComponent(id)}/continue-here`, {
    method: "POST",
  });
}

export function useSessionContext(id: string, active = true, refreshOnFocus = true) {
  const [loaded, setLoaded] = useState<{ id: string; data: SessionContext }>();
  const data = loaded?.id === id ? loaded.data : undefined;
  const [error, setError] = useState("");
  const reload = useCallback(async () => {
    try {
      invalidateRead(id);
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
    const apply = (promise: Promise<SessionContext>) => {
      void promise.then((next) => {
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
    const refresh = () => apply(readDisplaySessionContext(id));
    const refreshFresh = () => {
      invalidateRead(id);
      apply(loadSessionContext(id));
    };
    const local = (event: Event) => {
      const target = (event as CustomEvent<string | null>).detail;
      if (target === null || target === id) refresh();
    };
    const onFocus = () => {
      // Sessions with server-pushed preference announcements recover via
      // PREFERENCES_CHANGED on reconnect; ownerless sessions (product auth off)
      // have no owner stream and still revalidate when the tab regains focus.
      if (refreshOnFocus || !announcedRevisions.has(id)) refreshFresh();
    };
    refresh();
    window.addEventListener(CONTEXT_CHANGED, local);
    window.addEventListener("focus", onFocus);
    return () => {
      alive = false;
      window.removeEventListener(CONTEXT_CHANGED, local);
      window.removeEventListener("focus", onFocus);
    };
  }, [id, active, refreshOnFocus]);
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
  cachedContexts.clear();
  announcedRevisions.clear();
  loadedAnnouncedRevisions.clear();
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
      "Send from the Composer with the selected context, or cancel the pending context changes.",
    );
  }
}

const offeredUndo = new Set<string>();

function offerHandoffUndo(id: string, receipt: ContextDelivery): void {
  if (!receipt.source_trashed || offeredUndo.has(receipt.operation)) return;
  offeredUndo.add(receipt.operation);
  // Lazily: Undo reaches back into the store that imports this module.
  void import("./undoHistory").then(({ addUndoEntry }) =>
    addUndoEntry("Original moved to Trash", [{ kind: "restore_trash", sessions: [id] }])
  );
}

export async function sendWithContext(
  id: string,
  text: string,
  content: unknown[],
): Promise<boolean> {
  await linkEdits.get(id);
  // Ordinary sessions retain their offline outbox and zero extra fetches. The
  // owner-scoped preferences stream announces links across tabs and reconnects;
  // the Service also rejects stale direct sends at the dispatch boundary.
  if (!configured.has(id) && !sends.has(id)) return false;
  const context = await loadSessionContext(id);
  let request = sends.get(id);
  // Taken back on any device: the message returned to a Composer, so the
  // unresolved send is over and the next one is ordinary.
  if (
    request && context.delivery?.state === "cancelled" &&
    context.delivery.operation === request.operation
  ) {
    sends.delete(id);
    request = undefined;
  }
  if (
    !request && context.revision === context.applied_revision &&
    context.delivery?.state === "accepted" &&
    context.delivery.session_id !== id && !context.handoff_resumed &&
    !observedHandoffs.has(context.delivery.operation)
  ) {
    openContextDestination(context.delivery);
    offerHandoffUndo(id, context.delivery);
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
    offerHandoffUndo(id, receipt);
  }
  return true;
}
