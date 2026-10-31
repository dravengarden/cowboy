export const OPEN_SESSION_SETTINGS_EVENT = "cowboy:open-session-settings";
export const CLOSE_SESSION_SETTINGS_EVENT = "cowboy:close-session-settings";
export const SESSION_NEW_AGENT_EVENT = "cowboy:session-new-agent";
export const SESSION_DUPLICATE_EVENT = "cowboy:session-duplicate";
export const COMPOSER_FOCUS_EVENT = "cowboy:composer-focus";

export type SessionSettingsFocus = "session" | "agent";

/** A request made while another session was selected is honored by the
 *  target's settings surfaces when they mount, within a short window. */
const PENDING_MS = 3_000;
let pendingOpen:
  | { sessionId: string; focus: SessionSettingsFocus; at: number }
  | undefined;
const newAgentRequests = new Map<string, number>();
const duplicateRequests = new Map<string, number>();

export function openSessionSettings(
  focus: SessionSettingsFocus = "agent",
  sessionId?: string,
): void {
  pendingOpen = sessionId ? { sessionId, focus, at: Date.now() } : undefined;
  document.dispatchEvent(
    new CustomEvent(OPEN_SESSION_SETTINGS_EVENT, { detail: { focus, sessionId } }),
  );
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

/** Open this session's settings with "Continue with a new agent" selected. */
export function requestNewAgent(sessionId: string): void {
  newAgentRequests.set(sessionId, Date.now());
  document.dispatchEvent(
    new CustomEvent(SESSION_NEW_AGENT_EVENT, { detail: { sessionId } }),
  );
  openSessionSettings("agent", sessionId);
}

export function takeNewAgentRequest(sessionId: string): boolean {
  const at = newAgentRequests.get(sessionId);
  newAgentRequests.delete(sessionId);
  return at !== undefined && Date.now() - at < PENDING_MS;
}

/** Open this session's settings at Duplicate. */
export function requestDuplicate(sessionId: string): void {
  duplicateRequests.set(sessionId, Date.now());
  document.dispatchEvent(
    new CustomEvent(SESSION_DUPLICATE_EVENT, { detail: { sessionId } }),
  );
  openSessionSettings("agent", sessionId);
}

export function takeDuplicateRequest(sessionId: string): boolean {
  const at = duplicateRequests.get(sessionId);
  duplicateRequests.delete(sessionId);
  return at !== undefined && Date.now() - at < PENDING_MS;
}

export function requestComposerFocus(sessionId: string): void {
  document.dispatchEvent(
    new CustomEvent(COMPOSER_FOCUS_EVENT, { detail: { sessionId } }),
  );
}

export function sessionSettingsEventMatches(event: Event, sessionId: string): boolean {
  return !(event instanceof CustomEvent) || !event.detail?.sessionId ||
    event.detail.sessionId === sessionId;
}

export function sessionSettingsFocusFromEvent(
  event: Event,
): SessionSettingsFocus {
  if (!(event instanceof CustomEvent)) return "session";
  return event.detail?.focus === "agent" ? "agent" : "session";
}
