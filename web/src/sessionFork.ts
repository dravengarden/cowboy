import type { MachineSummary, SessionMeta } from "./protocol";

/** Providers whose adapter fork Cowboy verified; the server enforces the same. */
const FORKABLE_PROVIDERS = new Set(["claude-code", "codex"]);

export type SessionForkFetch = (
  input: string,
  init: RequestInit,
) => Promise<Response>;

export interface SessionForkReceipt {
  session_id: string;
  session?: SessionMeta | null;
}

/** How the session menu offers Fork, or `null` to hide it. */
export function forkAvailability(
  session: Pick<SessionMeta, "provider" | "status" | "machine_id" | "system">,
  machines: readonly Pick<MachineSummary, "id" | "capabilities">[],
): "ready" | "busy" | null {
  if (session.system || !FORKABLE_PROVIDERS.has(session.provider)) return null;
  const machine = machines.find((candidate) =>
    candidate.id === session.machine_id
  );
  if (!machine?.capabilities?.native_fork) return null;
  if (session.status === "busy" || session.status === "starting") {
    return "busy";
  }
  return "ready";
}

function operationId(): string {
  return `fork-${crypto.randomUUID()}`;
}

/**
 * Create a session that natively continues `sessionId`'s conversation, up to
 * and including `messageId` (a turn-final assistant message) when given.
 * Retrying with the same `operation` returns the same destination.
 */
export async function forkSession(
  sessionId: string,
  options: { messageId?: string; operation?: string } = {},
  fetcher: SessionForkFetch = globalThis.fetch,
): Promise<SessionForkReceipt> {
  const response = await fetcher(
    `/api/sessions/${encodeURIComponent(sessionId)}/fork`,
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        operation: options.operation ?? operationId(),
        ...(options.messageId ? { message_id: options.messageId } : {}),
      }),
    },
  );
  const text = await response.text();
  let body: unknown = null;
  try {
    body = text ? JSON.parse(text) : null;
  } catch {
    body = null;
  }
  const record = body !== null && typeof body === "object"
    ? body as Record<string, unknown>
    : {};
  if (response.ok && typeof record.session_id === "string") {
    return {
      session_id: record.session_id,
      session: (record.session ?? null) as SessionMeta | null,
    };
  }
  const error = typeof record.error === "string" ? record.error : text.trim();
  throw new Error(
    error || `Session fork failed (HTTP ${String(response.status)})`,
  );
}
