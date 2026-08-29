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

interface VerticalBounds {
  readonly top: number;
  readonly bottom: number;
}

/** Scroll distance needed to keep a focused setting between sticky chrome. */
export function settingsFocusRevealDelta(
  target: VerticalBounds,
  visible: VerticalBounds,
  gap = 12,
): number {
  const visibleTop = visible.top + gap;
  const visibleBottom = visible.bottom - gap;
  if (visibleBottom <= visibleTop) return 0;
  if (target.bottom > visibleBottom) return target.bottom - visibleBottom;
  if (target.top < visibleTop) return target.top - visibleTop;
  return 0;
}
