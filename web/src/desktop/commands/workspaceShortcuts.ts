import { ALT_LABEL, isMac } from "../../platform";
import { matchesShortcut, parseShortcut } from "./shortcut";
import { workspaceCommandKey } from "./workspaceCommandKey";

/**
 * The workspace prefix deliberately follows each platform's browser-safe path.
 * Chrome leaves Command-K available on macOS, while Ctrl-K focuses Search on
 * Windows/Linux, so those platforms use Alt-K instead.
 */
export function desktopWorkspacePrefix(mac: boolean): "Mod+K" | "Alt+K" {
  return mac ? "Mod+K" : "Alt+K";
}

export const DESKTOP_WORKSPACE_PREFIX = desktopWorkspacePrefix(isMac);

export const DESKTOP_WORKSPACE_KEYS = {
  focusSessions: "S",
  focusPrompt: "P",
  focusTopbar: "T",
  focusConversation: "C",
  focusPlan: "L",
  focusQueue: "Q",
  focusDrafts: "D",
  newSession: "N",
  cycleRegion: "W",
  resize: "R",
  settings: ",",
} as const;

export function desktopWorkspaceSequence(key: string): string {
  return `${DESKTOP_WORKSPACE_PREFIX} → ${key}`;
}

/** One source of truth for shortcut registration and every visible hint. */
export const DESKTOP_SHORTCUTS = {
  shortcuts: "Mod+/",
  commands: "Mod+Shift+P",
  stop: "Mod+.",
  saveDraft: "Mod+S",
  newSession: desktopWorkspaceSequence(DESKTOP_WORKSPACE_KEYS.newSession),
  settings: desktopWorkspaceSequence(DESKTOP_WORKSPACE_KEYS.settings),
  focusTopbar: desktopWorkspaceSequence(DESKTOP_WORKSPACE_KEYS.focusTopbar),
  focusSessions: desktopWorkspaceSequence(DESKTOP_WORKSPACE_KEYS.focusSessions),
  focusPrompt: desktopWorkspaceSequence(DESKTOP_WORKSPACE_KEYS.focusPrompt),
  focusConversation: desktopWorkspaceSequence(DESKTOP_WORKSPACE_KEYS.focusConversation),
  focusPlan: desktopWorkspaceSequence(DESKTOP_WORKSPACE_KEYS.focusPlan),
  focusQueue: desktopWorkspaceSequence(DESKTOP_WORKSPACE_KEYS.focusQueue),
  focusDrafts: desktopWorkspaceSequence(DESKTOP_WORKSPACE_KEYS.focusDrafts),
  cycleRegion: desktopWorkspaceSequence(DESKTOP_WORKSPACE_KEYS.cycleRegion),
  resize: desktopWorkspaceSequence(DESKTOP_WORKSPACE_KEYS.resize),
  sessionSlots: "Alt+1…0",
} as const;

export const DESKTOP_SESSION_SLOTS_LABEL = `${ALT_LABEL}${isMac ? "" : "+"}1…0`;

export const DESKTOP_FOCUS_PROMPT_SHORTCUT = DESKTOP_SHORTCUTS.focusPrompt;
export const DESKTOP_FOCUS_PLAN_SHORTCUT = DESKTOP_SHORTCUTS.focusPlan;
export const DESKTOP_RESIZE_SELECT_SHORTCUT = DESKTOP_SHORTCUTS.resize;
export const DESKTOP_RESIZE_HINT = DESKTOP_SHORTCUTS.resize;

/** One stable meaning for every workspace-prefix continuation. */
export const DESKTOP_WORKSPACE_COMMANDS: Readonly<Record<string, string>> = {
  s: "workspace.focusSessions",
  p: "workspace.focusPrompt",
  t: "workspace.focusTopbar",
  c: "workspace.focusConversation",
  l: "prompt.focusPlan",
  q: "prompt.focusQueue",
  d: "prompt.focusDrafts",
  n: "session.new",
  w: "workspace.cycleRegion",
  r: "workspace.enterResize",
  ",": "settings.open",
};

type WorkspaceKeyEvent = Pick<
  KeyboardEvent,
  "key" | "code" | "metaKey" | "ctrlKey" | "shiftKey" | "altKey"
>;

export function matchesDesktopWorkspacePrefix(
  event: WorkspaceKeyEvent,
  mac = isMac,
): boolean {
  return matchesShortcut(
    parseShortcut(desktopWorkspacePrefix(mac)),
    event,
    mac,
    true,
  );
}

/**
 * Return the physical continuation key while allowing the prefix modifier to
 * remain held. Any other modifier combination belongs to a fresh shortcut.
 */
export function desktopWorkspaceContinuationKey(
  event: WorkspaceKeyEvent,
  mac = isMac,
): string | null {
  const noPrefixModifier = !event.metaKey && !event.ctrlKey && !event.altKey;
  const prefixModifierHeld = mac
    ? event.metaKey && !event.ctrlKey && !event.altKey
    : event.altKey && !event.metaKey && !event.ctrlKey;
  if (!noPrefixModifier && !prefixModifierHeld) return null;
  return workspaceCommandKey(event);
}
