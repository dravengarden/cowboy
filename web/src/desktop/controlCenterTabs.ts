export const CONTROL_CENTER_TABS = [
  { value: "settings", label: "Settings", shortcut: "1" },
  { value: "providers", label: "Providers", shortcut: "2" },
  { value: "machines", label: "Machines", shortcut: "3" },
  { value: "info", label: "Info", shortcut: "4" },
  { value: "logs", label: "Logs", shortcut: "5" },
] as const;

export type ControlCenterTab = typeof CONTROL_CENTER_TABS[number]["value"];

export function controlCenterTabForShortcut(
  key: string,
): ControlCenterTab | null {
  return CONTROL_CENTER_TABS.find((tab) => tab.shortcut === key)?.value ?? null;
}

export function adjacentControlCenterTab(
  current: ControlCenterTab,
  direction: -1 | 1,
): ControlCenterTab {
  const index = CONTROL_CENTER_TABS.findIndex((tab) => tab.value === current);
  const next = (index + direction + CONTROL_CENTER_TABS.length) %
    CONTROL_CENTER_TABS.length;
  return CONTROL_CENTER_TABS[next]?.value ?? "settings";
}
