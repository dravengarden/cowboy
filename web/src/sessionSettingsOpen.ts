export const OPEN_SESSION_SETTINGS_EVENT = "cowboy:open-session-settings";
export const CLOSE_SESSION_SETTINGS_EVENT = "cowboy:close-session-settings";
export const SESSION_NEW_FROM_EVENT = "cowboy:session-new-from";
export const COMPOSER_FOCUS_EVENT = "cowboy:composer-focus";

export type SessionSettingsFocus = "session" | "agent" | "new_from";

/** A request made while another session was selected is honored by the
 *  target's settings surfaces when they mount, within a short window. */
const PENDING_MS = 3_000;
let pendingOpen:
  | { sessionId: string; focus: SessionSettingsFocus; at: number }
  | undefined;
const newFromRequests = new Map<string, number>();

export function openSessionSettings(
  focus: SessionSettingsFocus = "agent",
  sessionId?: string,
): void {
  pendingOpen = sessionId ? { sessionId, focus, at: Date.now() } : undefined;
  const untaken = document.dispatchEvent(
    new CustomEvent(OPEN_SESSION_SETTINGS_EVENT, {
      cancelable: true,
      detail: { focus, sessionId },
    }),
  );
  // A mounted surface took it; replaying it on a later remount within the
  // pending window would reopen settings the user had already closed.
  if (!untaken) pendingOpen = undefined;
}

export function pendingSessionSettings(
  sessionId: string,
): SessionSettingsFocus | null {
  return pendingOpen?.sessionId === sessionId &&
      Date.now() - pendingOpen.at < PENDING_MS
    ? pendingOpen.focus
    : null;
}

export function closeSessionSettings(sessionId: string): void {
  if (pendingOpen?.sessionId === sessionId) pendingOpen = undefined;
  document.dispatchEvent(
    new CustomEvent(CLOSE_SESSION_SETTINGS_EVENT, { detail: { sessionId } }),
  );
}

/** Open this session's settings at Replace or fork. */
export function requestNewFrom(sessionId: string): void {
  newFromRequests.set(sessionId, Date.now());
  document.dispatchEvent(
    new CustomEvent(SESSION_NEW_FROM_EVENT, { detail: { sessionId } }),
  );
  openSessionSettings("new_from", sessionId);
}

export function takeNewFromRequest(sessionId: string): boolean {
  const at = newFromRequests.get(sessionId);
  newFromRequests.delete(sessionId);
  return at !== undefined && Date.now() - at < PENDING_MS;
}

export function requestComposerFocus(sessionId: string): void {
  document.dispatchEvent(
    new CustomEvent(COMPOSER_FOCUS_EVENT, { detail: { sessionId } }),
  );
}

/** Whether a settings event targets this session's surfaces. A matching
 *  surface takes an open request (cancelable), so it is not kept pending. */
export function sessionSettingsEventMatches(event: Event, sessionId: string): boolean {
  const matches = !(event instanceof CustomEvent) || !event.detail?.sessionId ||
    event.detail.sessionId === sessionId;
  if (matches && event.cancelable) event.preventDefault();
  return matches;
}

export function sessionSettingsFocusFromEvent(
  event: Event,
): SessionSettingsFocus {
  if (!(event instanceof CustomEvent)) return "session";
  const focus = event.detail?.focus;
  return focus === "agent" || focus === "new_from" ? focus : "session";
}
