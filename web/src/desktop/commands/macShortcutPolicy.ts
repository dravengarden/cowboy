import { parseShortcut } from "./shortcut";

const SEMANTIC_MAC_SHORTCUTS = new Map<string, ReadonlySet<string>>([
  ["mod+n", new Set(["session.new"])],
  ["mod+,", new Set(["settings.open"])],
  ["mod+s", new Set(["composer.saveDraft"])],
  ["mod+e", new Set(["workspace.focusSessions"])],
  ["mod+p", new Set(["prompt.focusPlan"])],
  ["mod+i", new Set(["workspace.focusPrompt"])],
  ["mod+l", new Set(["workspace.focusConversation"])],
  ["mod+t", new Set(["workspace.focusTopbar"])],
  ["mod+k", new Set(["commandPalette.open"])],
  ["mod+.", new Set(["composer.more"])],
  ["mod+[", new Set(["workspace.resizeNarrow"])],
  ["mod+]", new Set(["workspace.resizeWiden"])],
  ["mod+\\", new Set(["workspace.enterResize"])],
]);

const RESERVED_MAC_SHORTCUTS = new Map<string, string>([
  ["mod+q", "Quit"],
  ["shift+mod+q", "Log Out"],
  ["mod+w", "Close window"],
  ["mod+h", "Hide app"],
  ["alt+mod+h", "Hide other apps"],
  ["mod+m", "Minimize window"],
  ["mod+tab", "Switch app"],
  ["mod+space", "Spotlight"],
  ["ctrl+mod+q", "Lock screen"],
  ["ctrl+mod+f", "Full screen"],
  ["alt+mod+escape", "Force Quit"],
  ["shift+mod+3", "Screenshot"],
  ["shift+mod+4", "Screenshot selection"],
  ["shift+mod+5", "Screenshot controls"],
  ["ctrl+space", "Previous input source"],
  ["ctrl+alt+space", "Next input source"],
  ["alt+`", "Grave-accent dead key"],
  ["alt+e", "Acute-accent dead key"],
  ["alt+i", "Circumflex dead key"],
  ["alt+n", "Tilde dead key"],
  ["alt+u", "Umlaut dead key"],
]);

const COMMON_APP_SHORTCUTS = new Map<string, string>([
  ["mod+a", "Select all"],
  ["mod+b", "Bold"],
  ["mod+c", "Copy"],
  ["mod+e", "Common editor/search action"],
  ["mod+f", "Find"],
  ["mod+i", "Italic"],
  ["mod+l", "Location/address field"],
  ["mod+o", "Open"],
  ["mod+p", "Print"],
  ["mod+r", "Reload"],
  ["mod+t", "New tab"],
  ["mod+u", "Underline/source"],
  ["mod+v", "Paste"],
  ["mod+x", "Cut"],
  ["mod+z", "Undo"],
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

export function macShortcutConflict(
  commandId: string,
  shortcut?: string,
): string | null {
  if (!shortcut) return null;
  const canonical = canonicalShortcut(shortcut);
  const semanticOwners = SEMANTIC_MAC_SHORTCUTS.get(canonical);
  if (semanticOwners) {
    return semanticOwners.has(commandId)
      ? null
      : `${shortcut} is reserved for a semantically matching native action`;
  }
  const reserved = RESERVED_MAC_SHORTCUTS.get(canonical);
  if (reserved) return `${shortcut} conflicts with macOS ${reserved}`;
  const common = COMMON_APP_SHORTCUTS.get(canonical);
  if (common) return `${shortcut} conflicts with the common ${common} action`;
  if (canonical === "q") {
    return "Bare Q can become Command-Q while the Command key is being released";
  }
  return null;
}

export function assertMacShortcutAllowed(
  commandId: string,
  shortcut?: string,
): void {
  const conflict = macShortcutConflict(commandId, shortcut);
  if (conflict) {
    throw new Error(`Unsafe Desktop shortcut for ${commandId}: ${conflict}`);
  }
}
