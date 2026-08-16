export type SettingsProductFocus = "agent" | "code";

export const OPEN_APP_SETTINGS_EVENT = "cowboy:open-app-settings";

export function openAppSettings(input?: {
  tab?: "settings" | "machines" | "info" | "logs";
  section?: SettingsProductFocus;
}): void {
  globalThis.dispatchEvent(
    new CustomEvent(OPEN_APP_SETTINGS_EVENT, { detail: input ?? {} }),
  );
}

export function appSettingsFromEvent(event: Event): {
  tab?: "settings" | "machines" | "info" | "logs";
  section?: SettingsProductFocus;
} {
  const detail = (event as CustomEvent<{
    tab?: "settings" | "machines" | "info" | "logs";
    section?: SettingsProductFocus;
  }>).detail;
  return detail ?? {};
}
