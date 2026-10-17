export const OPEN_SESSION_SETTINGS_EVENT = "cowboy:open-session-settings";

export type SessionSettingsFocus = "session" | "agent";

export function openSessionSettings(
  focus: SessionSettingsFocus = "agent",
  sessionId?: string,
): void {
  document.dispatchEvent(
    new CustomEvent(OPEN_SESSION_SETTINGS_EVENT, { detail: { focus, sessionId } }),
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
