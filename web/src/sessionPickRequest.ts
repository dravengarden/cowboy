// A surface outside the session app (the Mobile sync sheet, a notification)
// can ask the app to open one session exactly as a sessions-list tap would:
// the app owns navigation, drawer settling and the persisted active id.
export const PICK_SESSION_EVENT = "cowboy:pick-session";
import type { SessionMeta } from "./protocol";

export interface PickSessionDetail {
  readonly id: string;
  readonly session?: SessionMeta;
}

export function requestPickSession(id: string, session?: SessionMeta): void {
  globalThis.dispatchEvent(
    new CustomEvent<PickSessionDetail>(PICK_SESSION_EVENT, { detail: { id, ...(session ? { session } : {}) } }),
  );
}

export function pickSessionDetail(event: Event): string | null {
  const detail = (event as CustomEvent<Partial<PickSessionDetail>>).detail;
  return typeof detail?.id === "string" && detail.id !== "" ? detail.id : null;
}
