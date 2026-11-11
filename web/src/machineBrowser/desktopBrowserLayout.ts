/** Pure rules of the Desktop Machine browser modal: its size, the
 * remembered maximize choice, which Esc closes it, the chrome-release chord
 * and the header's quick-link overflow. See docs/machine-browser.md,
 * "Desktop panel". */
import { desktopKeyIntent } from "../desktop/commands/keyIntent";
import { matchesShortcut, parseShortcut } from "../desktop/commands/shortcut";

export const MAXIMIZED_KEY = "cowboy:machine-browser:maximized";

/** The modal's default box; maximized it fills the whole window. Large
 * enough for the 1920x1080 mode under the 40 px header on a big screen. */
export const MODAL_WIDTH = "min(96vw, 1960px)";
export const MODAL_HEIGHT = "min(94vh, 1180px)";

/** The header row's height (CSS px). */
export const HEADER_HEIGHT = 40;

/** The frame area (CSS px) of the modal in a `window`-sized viewport: the
 * box above less the header. The screen mode is chosen for this area. */
export function modalFrameSize(
  window: { width: number; height: number },
  maximized: boolean,
): { width: number; height: number } {
  const width = maximized ? window.width : Math.min(window.width * 0.96, 1960);
  const height = maximized ? window.height : Math.min(window.height * 0.94, 1180);
  return { width: Math.floor(width), height: Math.floor(height) - HEADER_HEIGHT };
}

export interface StorageLike {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

export function readMaximized(storage: StorageLike | undefined): boolean {
  try {
    return storage?.getItem(MAXIMIZED_KEY) === "1";
  } catch {
    return false;
  }
}

export function writeMaximized(storage: StorageLike | undefined, maximized: boolean): void {
  try {
    storage?.setItem(MAXIMIZED_KEY, maximized ? "1" : "0");
  } catch {
    // A private window without storage keeps the choice for this session.
  }
}

/** The modal Paper's box for the current choice. */
export function modalBox(maximized: boolean): {
  width: string;
  height: string;
  borderRadius: number;
} {
  return maximized
    ? { width: "100vw", height: "100vh", borderRadius: 0 }
    : { width: MODAL_WIDTH, height: MODAL_HEIGHT, borderRadius: 3 };
}

/** Whether an Esc closes the modal: from Cowboy's chrome, the empty page
 * and the Machine chooser included. With focus in the remote viewer, Esc
 * (like every key) belongs to the remote page. */
export function escapeClosesModal(focused: Element | null | undefined): boolean {
  return !focused?.closest("[data-machine-browser-viewer]");
}

/** Whether a key is the chrome-release chord (never during an IME
 * composition). */
export function isBrowserRelease(event: KeyboardEvent, shortcut: string, mac: boolean): boolean {
  if (desktopKeyIntent(event).owner === "ime") return false;
  return matchesShortcut(parseShortcut(shortcut), event, mac);
}
