import type {
  DesktopPane,
  DesktopSplitterId,
} from "./DesktopWorkspaceController";

export const DESKTOP_SPLITTER_ADJUST_EVENT =
  "cowboy:desktop-splitter-adjust";
export const DESKTOP_SPLITTER_STEP = 16;
export const DESKTOP_SPLITTER_LARGE_STEP = 48;

/** Every splitter Resize mode cycles: the work panes', then Review's
 *  Navigator edge (docs/desktop-code-review.md "Inner width policy"). */
export const DESKTOP_SPLITTER_IDS: readonly DesktopSplitterId[] = [
  "sessions-prompt",
  "prompt-conversation",
  "review-navigator",
];

export function isDesktopSplitterId(value: unknown): value is DesktopSplitterId {
  return typeof value === "string" && (DESKTOP_SPLITTER_IDS as readonly string[]).includes(value);
}

export interface DesktopSplitterAdjustment {
  splitter: DesktopSplitterId;
  delta: number;
}

export function splitterAdjustment(event: Event): DesktopSplitterAdjustment | null {
  if (!(event instanceof CustomEvent) || typeof event.detail !== "object" ||
    event.detail === null) return null;
  const detail = event.detail as Record<string, unknown>;
  const splitter = detail.splitter;
  const delta = detail.delta;
  if (!isDesktopSplitterId(splitter)) return null;
  return typeof delta === "number" && Number.isFinite(delta)
    ? { splitter, delta }
    : null;
}

export function visibleDesktopSplitterIds(
  root: ParentNode = document,
): DesktopSplitterId[] {
  return [...root.querySelectorAll<HTMLElement>("[data-desktop-splitter]")]
    .filter((element) => element.offsetParent !== null)
    .map((element) => element.dataset.desktopSplitter)
    .filter(isDesktopSplitterId);
}

export function preferredDesktopSplitter(
  visible: readonly DesktopSplitterId[],
  pane: DesktopPane,
): DesktopSplitterId | null {
  if (visible.length === 0) return null;
  if (pane === "sessions" && visible.includes("sessions-prompt")) {
    return "sessions-prompt";
  }
  if (visible.includes("prompt-conversation")) return "prompt-conversation";
  return visible[0] ?? null;
}

export function adjacentDesktopSplitter(
  visible: readonly DesktopSplitterId[],
  current: DesktopSplitterId,
  delta: -1 | 1,
): DesktopSplitterId | null {
  if (visible.length === 0) return null;
  const index = visible.indexOf(current);
  const currentIndex = index < 0 ? 0 : index;
  return visible[(currentIndex + delta + visible.length) % visible.length] ?? null;
}

export function resolveDesktopResizeSplitter(
  visible: readonly DesktopSplitterId[],
  selected: DesktopSplitterId | null,
  pane: DesktopPane,
): DesktopSplitterId | null {
  if (selected && visible.includes(selected)) return selected;
  return preferredDesktopSplitter(visible, pane);
}
