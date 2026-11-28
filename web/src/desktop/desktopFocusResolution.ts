// Visibility-aware Desktop focus region resolution.
//
// Ensures keyboard pane hops (`Ctrl+H/L`, `Ctrl+J` from Top Bar, pane uncollapse,
// tab clicks) always land on a VISIBLE region inside the target pane (e.g.
// `review.document` or `tasks.reader` when Conversation is showing Review or
// Tasks) rather than stranding focus on a hidden `display: none` sibling.

import { desktopDefaultRegionForPane } from "./desktopComposerOwnership";
import type { DesktopPane } from "./DesktopWorkspaceController";

const PREFERRED_PANE_REGIONS: Record<DesktopPane, readonly string[]> = {
  sessions: ["sessions.rail", "sessions.list"],
  prompt: ["prompt.composer", "prompt.plan", "prompt.queued", "prompt.draft", "prompt.calls"],
  conversation: [
    "conversation.transcript",
    "review.document",
    "review.navigator",
    "tasks.reader",
    "tasks.roster",
  ],
};

/** True when `element` is actually rendered and visible in layout (not
 *  `hidden`, not `display: none`, not inside a hidden/collapsed ancestor). */
export function isDesktopRegionVisible(element: HTMLElement | null): boolean {
  if (!element || !element.isConnected) return false;
  if (element.hidden || element.closest("[hidden]")) return false;
  if (typeof globalThis.getComputedStyle === "function") {
    const style = globalThis.getComputedStyle(element);
    if (style.display === "none" || style.visibility === "hidden") return false;
    if (element.offsetParent === null && style.position !== "fixed") {
      // Check whether any ancestor is hidden via display:none; in JSDOM
      // offsetParent is always null even for visible elements.
      let cur: HTMLElement | null = element.parentElement;
      while (cur) {
        if (cur.hidden) return false;
        if (globalThis.getComputedStyle(cur).display === "none") return false;
        cur = cur.parentElement;
      }
    }
    return true;
  }
  return true;
}

/** Return the visible DOM element for `region`, preferring a visible match when
 *  multiple elements share a region name (falling back to the first match). */
export function resolveVisibleRegionElement(
  region: string,
  scope: ParentNode = document,
): HTMLElement | null {
  const matches = [
    ...scope.querySelectorAll<HTMLElement>(
      `[data-desktop-region="${CSS.escape(region)}"]`,
    ),
  ];
  if (matches.length === 0) return null;
  return matches.find((el) => isDesktopRegionVisible(el)) ?? matches[0] ?? null;
}

/** Return all visible `[data-desktop-region]` elements owned by `pane`. */
export function visibleRegionsInPane(
  pane: DesktopPane,
  scope: ParentNode = document,
): HTMLElement[] {
  const panes = [
    ...scope.querySelectorAll<HTMLElement>(`[data-desktop-pane="${pane}"]`),
  ].filter((el) => isDesktopRegionVisible(el));
  const regions: HTMLElement[] = [];
  for (const paneEl of panes) {
    if (paneEl.dataset.desktopRegion && isDesktopRegionVisible(paneEl)) {
      regions.push(paneEl);
    }
    for (const child of paneEl.querySelectorAll<HTMLElement>("[data-desktop-region]")) {
      if (isDesktopRegionVisible(child) && !regions.includes(child)) {
        regions.push(child);
      }
    }
  }
  return regions;
}

/** Pick the visible region to focus when entering `pane`, honoring the last
 *  focused sub-region in that pane if it is still visible, then falling back
 *  through the pane's preferred visible regions. */
export function resolvePaneEntryRegion(
  pane: DesktopPane,
  remembered?: string | null,
  scope: ParentNode = document,
): string | null {
  const visible = visibleRegionsInPane(pane, scope);
  if (remembered && visible.some((el) => el.dataset.desktopRegion === remembered)) {
    return remembered;
  }
  for (const candidate of PREFERRED_PANE_REGIONS[pane]) {
    if (visible.some((el) => el.dataset.desktopRegion === candidate)) {
      return candidate;
    }
  }
  const firstVisible = visible[0]?.dataset.desktopRegion;
  if (firstVisible) return firstVisible;
  return desktopDefaultRegionForPane(pane);
}

/** Resolve the region targeted by a pointer event: direct region ancestor first,
 *  otherwise the visible entry region of the clicked pane (never a hidden tab). */
export function resolveVisiblePointerRegion(
  target: EventTarget | null,
  rememberedByPane?: Partial<Record<DesktopPane, string | null>>,
  scope: ParentNode = document,
): string | null {
  if (!(target instanceof Element)) return null;
  const direct = target.closest<HTMLElement>("[data-desktop-region]")?.dataset.desktopRegion;
  if (direct) return direct;
  const paneValue = target.closest<HTMLElement>("[data-desktop-pane]")?.dataset.desktopPane;
  if (paneValue === "sessions" || paneValue === "prompt" || paneValue === "conversation") {
    return resolvePaneEntryRegion(paneValue, rememberedByPane?.[paneValue], scope);
  }
  return null;
}
