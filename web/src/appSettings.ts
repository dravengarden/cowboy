import type { ControlCenterTab } from "./desktop/controlCenterTabs";

export type SettingsProductFocus = "agent" | "code";

export const OPEN_APP_SETTINGS_EVENT = "cowboy:open-app-settings";

export function openAppSettings(input?: {
  tab?: ControlCenterTab;
  section?: SettingsProductFocus;
}): void {
  globalThis.dispatchEvent(
    new CustomEvent(OPEN_APP_SETTINGS_EVENT, { detail: input ?? {} }),
  );
}

export function appSettingsFromEvent(event: Event): {
  tab?: ControlCenterTab;
  section?: SettingsProductFocus;
} {
  const detail = (event as CustomEvent<{
    tab?: ControlCenterTab;
    section?: SettingsProductFocus;
  }>).detail;
  return detail ?? {};
}
