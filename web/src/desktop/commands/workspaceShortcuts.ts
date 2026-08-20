import { ALT_LABEL, isMac } from "../../platform";

/** One source of truth for shortcut registration and every visible hint. */
export const DESKTOP_SHORTCUTS = {
  shortcuts: "?",
  commands: ":",
  newSession: "N",
  settings: ",",
  focusTopbar: "T",
  focusSessions: "S",
  focusPrompt: "E",
  focusConversation: "C",
  focusPlan: "P",
  focusQueue: "Y",
  focusDrafts: "D",
  cycleRegion: "W",
  resize: "\\",
  sessionSlots: "Alt+1…0",
} as const;

export const DESKTOP_SESSION_SLOTS_LABEL = `${ALT_LABEL}${isMac ? "" : "+"}1…0`;

export const DESKTOP_FOCUS_PROMPT_SHORTCUT = DESKTOP_SHORTCUTS.focusPrompt;
export const DESKTOP_FOCUS_PLAN_SHORTCUT = DESKTOP_SHORTCUTS.focusPlan;
export const DESKTOP_RESIZE_SELECT_SHORTCUT = DESKTOP_SHORTCUTS.resize;
export const DESKTOP_RESIZE_HINT = DESKTOP_SHORTCUTS.resize;

/** Fired by a plain-Normal editor Escape to arm one subsequent workspace key. */
export const DESKTOP_WORKSPACE_COMMAND_EVENT = "cowboy:desktop-workspace-command";

/** Commands accepted by the one-shot editor-to-workspace command state. */
export const DESKTOP_WORKSPACE_COMMANDS: Readonly<Record<string, string>> = {
  "?": "shortcuts.open",
  ":": "commandPalette.open",
  n: "session.new",
  ",": "settings.open",
  t: "workspace.focusTopbar",
  s: "workspace.focusSessions",
  e: "workspace.focusPrompt",
  c: "workspace.focusConversation",
  p: "prompt.focusPlan",
  y: "prompt.focusQueue",
  d: "prompt.focusDrafts",
  w: "workspace.cycleRegion",
  "\\": "workspace.enterResize",
};
