import type { MachineSummary, SessionMeta } from "./protocol";

export type SessionHibernateFetch = (
  input: string,
  init: RequestInit,
) => Promise<Response>;

/** How the session menu offers hibernation, or `null` to hide it. */
export function hibernateAvailability(
  session: Pick<SessionMeta, "status" | "machine_id">,
  machines: readonly Pick<MachineSummary, "id" | "capabilities">[],
): "ready" | "busy" | null {
  const machine = machines.find((candidate) => candidate.id === session.machine_id);
  if (!machine?.capabilities?.hibernation) return null;
  if (session.status === "running") return "ready";
  if (session.status === "busy" || session.status === "starting") return "busy";
  return null;
}

/** Free an idle session's Agent process and Machine slot; it resumes on use. */
export async function hibernateSession(
  sessionId: string,
  fetcher: SessionHibernateFetch = globalThis.fetch,
): Promise<void> {
  const response = await fetcher(
    `/api/sessions/${encodeURIComponent(sessionId)}/hibernate`,
    { method: "POST" },
  );
  if (response.ok) return;
  const detail = (await response.text()).trim();
  throw new Error(
    detail || `Session hibernation failed (HTTP ${String(response.status)})`,
  );
}
