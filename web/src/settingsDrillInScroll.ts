import type { ControlCenterTab } from "./desktop/controlCenterTabs";

/** Remember the Settings list offset only when leaving that list. */
export function nextSavedSettingsScroll(
  leavingTab: ControlCenterTab,
  currentScroll: number,
  previousSaved: number,
): number {
  return leavingTab === "settings" ? Math.max(0, currentScroll) : previousSaved;
}

/** Drill-in pages start at the top; returning to Settings restores the list. */
export function destinationScrollTop(
  enteringTab: ControlCenterTab,
  savedSettingsScroll: number,
): number {
  return enteringTab === "settings" ? Math.max(0, savedSettingsScroll) : 0;
}

export function closestScrollableSettingsSurface(panel: HTMLElement): HTMLElement {
  let candidate: HTMLElement | null = panel;
  while (candidate) {
    const style = globalThis.getComputedStyle(candidate);
    if (
      /(auto|scroll)/.test(style.overflowY) &&
      candidate.scrollHeight > candidate.clientHeight
    ) return candidate;
    candidate = candidate.parentElement;
  }
  return panel;
}
