import { matchesShortcut, parseShortcut } from "./shortcut";

/**
 * Browser/PWA chords that open Chrome chrome Cowboy does not own.
 *
 * Do not put OS window chords here (Quit, Close, Hide, Minimize, Spotlight).
 * Do not put native edit chords (Cut/Copy/Paste/Select all/Undo) or recovery
 * chords (Reload, zoom, DevTools). Cowboy-owned Mod+letter commands already
 * preventDefault when they run; this list is only the leftovers that would
 * otherwise surface a browser Find bar, Downloads page, or file picker.
 */
const SHIELDED_BROWSER_SHORTCUTS = [
  "Mod+F",
  "Shift+Mod+F",
  "Mod+G",
  "Shift+Mod+G",
  "Mod+O",
  "Mod+U",
  "Mod+J",
] as const;

const WINDOWS_HISTORY_SHORTCUTS = [
  "Alt+ArrowLeft",
  "Alt+ArrowRight",
] as const;

export function desktopBrowserChromeShortcut(
  event: Parameters<typeof matchesShortcut>[1],
  mac: boolean,
): boolean {
  for (const shortcut of SHIELDED_BROWSER_SHORTCUTS) {
    if (matchesShortcut(parseShortcut(shortcut), event, mac)) return true;
  }
  if (!mac) {
    for (const shortcut of WINDOWS_HISTORY_SHORTCUTS) {
      if (matchesShortcut(parseShortcut(shortcut), event, mac)) return true;
    }
  }
  return false;
}
