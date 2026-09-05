export type SessionReloadFetch = (
  input: string,
  init: RequestInit,
) => Promise<Response>;

export interface SessionReloadOptions {
  confirmActiveTurn?: boolean;
  providerGenerationDigest?: string;
}

export interface SessionReloadPlan {
  current_version: string;
  target_version?: string;
  target_digest?: string;
  upgrade_available: boolean;
  blocked_reason?: string;
}

export async function loadSessionReloadPlan(
  sessionId: string,
  fetcher: SessionReloadFetch = globalThis.fetch,
): Promise<SessionReloadPlan> {
  const response = await fetcher(
    `/api/sessions/${encodeURIComponent(sessionId)}/reload`,
    {
      method: "GET",
      cache: "no-store",
    },
  );
  if (!response.ok) {
    throw new Error(
      "Could not check the installed Provider version. You can still reload the pinned version.",
    );
  }
  const value: unknown = await response.json();
  if (
    typeof value !== "object" || value === null ||
    !("current_version" in value) ||
    typeof value.current_version !== "string" ||
    !("upgrade_available" in value) ||
    typeof value.upgrade_available !== "boolean" ||
    (value.upgrade_available &&
      (!("target_version" in value) ||
        typeof value.target_version !== "string" ||
        !("target_digest" in value) || typeof value.target_digest !== "string"))
  ) {
    throw new Error(
      "Invalid session reload plan; keeping the pinned Provider version.",
    );
  }
  return value as SessionReloadPlan;
}

/** Reload a session runtime while keeping its durable Cowboy/native session. */
export async function reloadSession(
  sessionId: string,
  options: SessionReloadOptions = {},
  fetcher: SessionReloadFetch = globalThis.fetch,
): Promise<void> {
  const query = new URLSearchParams();
  if (options.confirmActiveTurn === true) {
    query.set("confirm_active_turn", "true");
  }
  if (options.providerGenerationDigest) {
    query.set("upgrade_provider", "true");
    query.set("expected_generation_digest", options.providerGenerationDigest);
  }
  const suffix = query.size > 0 ? `?${query.toString()}` : "";
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
