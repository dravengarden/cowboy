import { parseShortcut } from "./shortcut";
import { isMac } from "../../platform";

const SEMANTIC_CHROME_SHORTCUTS = new Map<string, ReadonlySet<string>>([
  ["mod+s", new Set(["composer.saveDraft"])],
]);

/**
 * Chrome-owned chords which a Cowboy command must not claim by default.
 *
 * Mod means Command on macOS and Control on Windows/Linux. The inventory is
 * intentionally broader than the chords Chrome makes impossible to cancel:
 * cancelable browser behavior is still user-visible behavior and needs an
 * explicit semantic exception above.
 */
const CHROME_SHORTCUTS = new Map<string, string>([
  ["mod+n", "New window"],
  ["shift+mod+n", "New incognito window"],
  ["mod+t", "New tab"],
  ["shift+mod+t", "Reopen closed tab"],
  ["mod+w", "Close tab"],
  ["shift+mod+w", "Close window"],
  ["mod+tab", "Next tab"],
  ["shift+mod+tab", "Previous tab"],
  ["mod+0", "Reset zoom"],
  ["mod+1", "Select tab 1"],
  ["mod+2", "Select tab 2"],
  ["mod+3", "Select tab 3"],
  ["mod+4", "Select tab 4"],
  ["mod+5", "Select tab 5"],
  ["mod+6", "Select tab 6"],
  ["mod+7", "Select tab 7"],
  ["mod+8", "Select tab 8"],
  ["mod+9", "Select last tab"],
  ["mod+l", "Focus address bar"],
  ["mod+,", "Open Settings"],
  ["mod+[", "Back (macOS)"],
  ["mod+]", "Forward (macOS)"],
  ["mod+e", "Search from the address bar"],
  ["mod+p", "Print"],
  ["mod+s", "Save page"],
  ["mod+r", "Reload"],
  ["shift+mod+r", "Hard reload"],
  ["mod+f", "Find"],
  ["mod+g", "Next find result"],
  ["shift+mod+g", "Previous find result"],
  ["mod+h", "History"],
  ["mod+j", "Downloads"],
  ["mod+o", "Open file"],
  ["mod+u", "View source"],
  ["mod+d", "Bookmark page"],
  ["shift+mod+d", "Bookmark all tabs"],
  ["shift+mod+b", "Toggle bookmarks bar"],
  ["shift+mod+o", "Bookmark manager"],
  ["shift+mod+j", "Developer tools"],
  ["alt+mod+i", "Developer tools (macOS)"],
  ["alt+mod+j", "JavaScript console (macOS)"],
  ["alt+mod+u", "View source (macOS)"],
  ["alt+mod+f", "Search the web (macOS)"],
  ["alt+mod+p", "Page setup (macOS)"],
  ["alt+mod+b", "Bookmark manager (macOS)"],
  ["alt+mod+arrowleft", "Previous tab (macOS)"],
  ["alt+mod+arrowright", "Next tab (macOS)"],
  ["alt+mod+arrowup", "Chrome toolbar (macOS)"],
  ["alt+mod+arrowdown", "Chrome toolbar (macOS)"],
  ["alt+d", "Focus address bar"],
  ["alt+arrowleft", "Back"],
  ["alt+arrowright", "Forward"],
  ["alt+home", "Home page"],
  ["alt+f", "Chrome menu"],
  ["alt+e", "Chrome menu"],
  ["shift+escape", "Chrome Task Manager"],
  ["shift+mod+delete", "Delete browsing data"],
  ["f1", "Chrome Help"],
  ["f3", "Find"],
  ["f5", "Reload"],
  ["f6", "Browser chrome focus"],
  ["f7", "Caret browsing"],
  ["f10", "Chrome toolbar"],
  ["f11", "Full screen"],
  ["f12", "Developer tools"],
]);

function canonicalShortcut(shortcut: string): string {
  const stroke = parseShortcut(shortcut);
  return [
    stroke.ctrl ? "ctrl" : null,
    stroke.alt ? "alt" : null,
    stroke.shift ? "shift" : null,
    stroke.mod ? "mod" : null,
    stroke.key === "esc" ? "escape" : stroke.key,
  ].filter(Boolean).join("+");
}

export function chromeShortcutConflict(
  commandId: string,
  shortcut?: string,
  mac = isMac,
): string | null {
  if (!shortcut) return null;
  const canonical = canonicalShortcut(shortcut);
  const semanticOwners = SEMANTIC_CHROME_SHORTCUTS.get(canonical);
  if (semanticOwners) {
    return semanticOwners.has(commandId)
      ? null
      : `${shortcut} needs a matching application semantic`;
  }
  // Chromium registers Ctrl-K as an omnibox search shortcut only outside
  // macOS. Chrome on macOS leaves Command-K to the page/application.
  if (canonical === "mod+k" && !mac) {
    return `${shortcut} conflicts with Chrome Search from the address bar`;
  }
  const chromeAction = CHROME_SHORTCUTS.get(canonical);
  return chromeAction ? `${shortcut} conflicts with Chrome ${chromeAction}` : null;
}

export function assertChromeShortcutAllowed(
  commandId: string,
  shortcut?: string,
  mac = isMac,
): void {
  const conflict = chromeShortcutConflict(commandId, shortcut, mac);
  if (conflict) {
    throw new Error(`Unsafe Desktop shortcut for ${commandId}: ${conflict}`);
  }
}

/** Contextual Vim bindings deliberately preferred over Chrome in a reader. */
export const INTENTIONAL_CHROME_VIM_OVERRIDES = [
  "Ctrl+D",
  "Ctrl+U",
  "Ctrl+F",
  "Ctrl+B",
] as const;
