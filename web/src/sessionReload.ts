export type SessionReloadFetch = (
  input: string,
  init: RequestInit,
) => Promise<Response>;

export interface SessionReloadOptions {
  confirmActiveTurn?: boolean;
}

/** Reload a session runtime while keeping its durable Cowboy/native session. */
export async function reloadSession(
  sessionId: string,
  options: SessionReloadOptions = {},
  fetcher: SessionReloadFetch = globalThis.fetch,
): Promise<void> {
  const suffix = options.confirmActiveTurn === true
    ? "?confirm_active_turn=true"
    : "";
  const response = await fetcher(
    `/api/sessions/${encodeURIComponent(sessionId)}/reload${suffix}`,
    { method: "POST" },
  );
  if (response.ok) return;

  const detail = (await response.text()).trim();
  throw new Error(
    detail || `Session reload failed (HTTP ${String(response.status)})`,
  );
}
