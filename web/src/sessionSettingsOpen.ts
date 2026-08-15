export const OPEN_SESSION_SETTINGS_EVENT = "cowboy:open-session-settings";

export type SessionSettingsFocus = "session" | "agent";

export function openSessionSettings(
  focus: SessionSettingsFocus = "agent",
): void {
  document.dispatchEvent(
    new CustomEvent(OPEN_SESSION_SETTINGS_EVENT, { detail: { focus } }),
  );
}

export function sessionSettingsFocusFromEvent(
  event: Event,
): SessionSettingsFocus {
  if (!(event instanceof CustomEvent)) return "session";
  return event.detail?.focus === "agent" ? "agent" : "session";
}
