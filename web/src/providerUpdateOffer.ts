import type { ProviderUpdateAvailable, SessionMeta } from "./protocol";
import type { SessionReloadFetch } from "./sessionReload";

/** The newer installed Provider release this session can adopt, or `null`
 *  while none is waiting, an update is already running, or the session is a
 *  view-only system session. */
export function sessionProviderUpdate(
  session: SessionMeta | null | undefined,
): ProviderUpdateAvailable | null {
  if (!session || session.system || session.provider_update) return null;
  const offer = session.provider_update_available;
  if (!offer || offer.digest === session.provider_generation_digest) {
    return null;
  }
  return offer;
}

/** A busy session keeps its turn: the update can only be queued for later. */
export function providerUpdateWaitsForTurn(
  session: SessionMeta | null | undefined,
): boolean {
  return session?.status === "busy" || session?.status === "starting";
}

/** A workerless session is re-pinned instead of started, so updating it
 *  never spends a Device slot; its next open runs the new release. */
export function providerUpdateRepinsOnly(
  session: SessionMeta | null | undefined,
): boolean {
  return session?.status === "exited";
}

function approximateDuration(ms: number): string {
  const minutes = Math.max(1, Math.round(ms / 60_000));
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.round(minutes / 6) / 10;
  return `${Number.isInteger(hours) ? hours.toFixed(0) : hours.toFixed(1)} h`;
}

/** A short phrase about when the update happens without further action. */
export function providerUpdateScheduleText(
  offer: ProviderUpdateAvailable,
  waitsForTurn: boolean,
  nowMs: number,
): string | null {
  if (offer.when_idle) {
    return waitsForTurn ? "after this turn" : "starting shortly";
  }
  if (offer.automatic_at_ms === undefined) return null;
  const remaining = offer.automatic_at_ms - nowMs;
  if (remaining <= 60_000) return "auto when idle";
  return `auto after ~${approximateDuration(remaining)} idle`;
}

/** Ask the Controller to apply the offered release once the session is idle,
 *  or withdraw that request. Persisted on the Controller. */
export async function requestProviderUpdateWhenIdle(
  sessionId: string,
  whenIdle: boolean,
  fetcher: SessionReloadFetch = globalThis.fetch,
): Promise<void> {
  const response = await fetcher(
    `/api/sessions/${encodeURIComponent(sessionId)}/reload`,
    {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ when_idle: whenIdle }),
    },
  );
  if (!response.ok) {
    throw new Error(
      (await response.text()).trim() || "Could not schedule the update",
    );
  }
}
