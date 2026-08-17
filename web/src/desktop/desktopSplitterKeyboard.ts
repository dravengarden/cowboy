import type {
  DesktopPane,
  DesktopProductMode,
  DesktopSplitterId,
} from "./DesktopWorkspaceController";
import { workspaceCommandKey } from "./commands/workspaceCommandKey";

export const DESKTOP_SPLITTER_ADJUST_EVENT =
  "cowboy:desktop-splitter-adjust";
export const DESKTOP_SPLITTER_STEP = 16;
export const DESKTOP_SPLITTER_LARGE_STEP = 48;

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
  if (
    splitter !== "sessions-prompt" && splitter !== "prompt-conversation" &&
    splitter !== "questions-page"
  ) return null;
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
    .filter((id): id is DesktopSplitterId =>
      id === "sessions-prompt" || id === "prompt-conversation" ||
      id === "questions-page"
    );
}

export function preferredDesktopSplitter(
  visible: readonly DesktopSplitterId[],
  pane: DesktopPane,
  productMode: DesktopProductMode,
): DesktopSplitterId | null {
  if (visible.length === 0) return null;
  if (productMode === "reading" && visible.includes("questions-page")) {
    return "questions-page";
  }
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
  productMode: DesktopProductMode,
): DesktopSplitterId | null {
  if (selected && visible.includes(selected)) return selected;
  return preferredDesktopSplitter(visible, pane, productMode);
}

/** Mod+\ enters layout Resize mode without moving the bar. */
export function desktopResizeSelectChord(
  event: {
    code: string;
    key: string;
    shiftKey: boolean;
    metaKey: boolean;
    ctrlKey: boolean;
    altKey: boolean;
  },
  mac: boolean,
): boolean {
  const usesMod = mac
    ? event.metaKey && !event.ctrlKey
    : event.ctrlKey && !event.metaKey;
  if (!usesMod || event.altKey || event.shiftKey) return false;
  return workspaceCommandKey(event) === "\\";
}

/** Mod+[ narrows and Mod+] widens the nearest visible vertical boundary. */
export function desktopWidthResizeDirection(
  event: {
    code: string;
    key: string;
    shiftKey: boolean;
    metaKey: boolean;
    ctrlKey: boolean;
    altKey: boolean;
  },
  mac: boolean,
): -1 | 1 | null {
  const usesMod = mac
    ? event.metaKey && !event.ctrlKey
    : event.ctrlKey && !event.metaKey;
  if (!usesMod || event.altKey || event.shiftKey) return null;
  const key = workspaceCommandKey(event);
  if (key === "[") return -1;
  if (key === "]") return 1;
  return null;
}
