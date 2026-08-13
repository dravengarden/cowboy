export type SessionReloadFetch = (
  input: string,
  init: RequestInit,
) => Promise<Response>;

/** Reload a session runtime while keeping its durable Cowboy/native session. */
export async function reloadSession(
  sessionId: string,
  fetcher: SessionReloadFetch = globalThis.fetch,
): Promise<void> {
  const response = await fetcher(
    `/api/sessions/${encodeURIComponent(sessionId)}/reload`,
    { method: "POST" },
  );
  if (response.ok) return;

  const detail = (await response.text()).trim();
  throw new Error(
    detail || `Session reload failed (HTTP ${String(response.status)})`,
  );
}
