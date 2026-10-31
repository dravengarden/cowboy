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
  options: {
    messageId?: string;
    operation?: string;
    /** Live options (model, reasoning, …) the copy uses instead of the inherited ones. */
    config?: Record<string, string | boolean>;
  } = {},
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
        ...(options.config && Object.keys(options.config).length > 0
          ? { config: options.config }
          : {}),
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

/** How Duplicate copies a session: a native fork keeps the whole native
 *  conversation (same Provider and subscription, where the Machine supports
 *  it); otherwise a new agent reads the source as linked context. */
export function duplicateMode(input: {
  sourceProvider: string;
  sourceAccount: string;
  provider: string;
  account: string;
  forkAvailable: boolean;
}): "fork" | "linked" {
  return input.forkAvailable && input.provider === input.sourceProvider &&
      input.account === input.sourceAccount
    ? "fork"
    : "linked";
}

/** A copy on another subscription or Provider (or without native fork). The
 *  idempotency key makes a retried request return the same copy. */
export async function duplicateSession(
  sessionId: string,
  body: {
    provider: string;
    provider_account_id: string;
    preset?: string | null;
    config?: Record<string, string | boolean>;
    inherit_settings?: boolean;
  },
  key: string,
  fetcher: SessionForkFetch = globalThis.fetch,
): Promise<SessionForkReceipt> {
  const response = await fetcher(
    `/api/sessions/${encodeURIComponent(sessionId)}/duplicate`,
    {
      method: "POST",
      headers: { "content-type": "application/json", "idempotency-key": key },
      body: JSON.stringify({
        operation: key,
        provider: body.provider,
        provider_account_id: body.provider_account_id,
        ...(body.preset ? { preset: body.preset } : {}),
        ...(body.config && Object.keys(body.config).length > 0
          ? { config: body.config }
          : {}),
        ...(body.inherit_settings === false ? { inherit_settings: false } : {}),
      }),
    },
  );
  const text = await response.text();
  let record: Record<string, unknown> = {};
  try {
    const parsed: unknown = text ? JSON.parse(text) : null;
    if (parsed !== null && typeof parsed === "object") record = parsed as Record<string, unknown>;
  } catch {
    record = {};
  }
  if (response.ok && typeof record.session_id === "string") {
    return {
      session_id: record.session_id,
      session: (record.session ?? null) as SessionMeta | null,
    };
  }
  const error = typeof record.error === "string" ? record.error : text.trim();
  throw new Error(error || `Duplicate failed (HTTP ${String(response.status)})`);
}
