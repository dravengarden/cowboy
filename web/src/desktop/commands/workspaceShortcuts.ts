import { isMac } from "../../platform";
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

/**
 * The leader (FOCUS.md "Leader"). Space arms it wherever Cowboy owns the key
 * (Vim Normal, lists, readers, chrome); the platform prefix above arms the
 * same layer from Insert and native fields. Every continuation is drawn as
 * one keycap: the leader glyph plus its key.
 */
export const DESKTOP_LEADER_GLYPH = "␣";

export function desktopLeaderLabel(key: string): string {
  return `${DESKTOP_LEADER_GLYPH}${key === " " ? DESKTOP_LEADER_GLYPH : key.toUpperCase()}`;
}

/** which-key groups: `␣` + group key opens a layer of related commands. */
export const DESKTOP_LEADER_GROUPS: Readonly<Record<string, string>> = {
  t: "Top bar",
  // Editor formatting (direct chords such as Mod+B remain).
  m: "Markup",
  // Layout: cycle regions, resize, fold panes.
  w: "Window",
  // Presentation toggles.
  u: "Interface",
  // Only while a Draft document is open; otherwise `␣D` focuses Drafts.
  d: "Draft",
};

/** The `␣D` group of an open Draft document (FOCUS.md "Draft document"). */
export const DESKTOP_DRAFT_GROUP_KEYS = {
  group: "D",
  rename: "R",
  copy: "V",
  history: "H",
  export: "E",
  readableWidth: "W",
} as const;

/** Labels for the `␣␣` session switcher: home row first, then the rest. */
export const DESKTOP_JUMP_LABELS = "asdfghjklqwertyuiopzxcvbnm";

/**
 * Leader paths: one character is a root key (`␣N`), two are a group and its
 * key (`␣MB`). Every slot, sequence and help row derives from these.
 */
export const DESKTOP_WORKSPACE_KEYS = {
  switchSession: " ",
  alternateSession: "`",
  commandPalette: "K",
  focusSessions: "S",
  focusPrompt: "P",
  focusTopbar: "T",
  focusConversation: "C",
  focusPlan: "L",
  focusQueue: "Q",
  focusDrafts: "D",
  newSession: "N",
  cycleRegion: "WW",
  resize: "WR",
  settings: ",",
  // Obsidian binds live-preview ↔ source to Mod+E, which Cowboy cannot have:
  // Chrome owns it for the address bar and macOS apps for a common editor
  // action (chromeShortcutPolicy / macShortcutPolicy both reject it). The
  // workspace prefix is FOCUS.md's documented fallback, and it keeps the same
  // E mnemonic while working from Vim Insert, Normal and native inputs.
  toggleSourceMode: "UE",
  composerSlash: "/",
  composerReference: "F",
  composerAttach: "A",
  composerSchedule: "H",
  composerJumpFront: "J",
  composerMore: "MM",
  // Zoom the focused editor into the fullscreen composer.
  editorExpand: "Z",
  // Pane collapse uses three adjacent physical keys whose left-to-right order
  // matches the panes on screen: Sessions | Prompt | Conversation, inside the
  // Window group (`␣W[` `␣W]` `␣W\\`).
  toggleSessions: "W[",
  togglePrompt: "W]",
  toggleConversation: "W\\",
} as const;

/** Formatting shares the leader; bare letters remain editor input. Undo and
 *  redo stay with the editor (`u`/`Ctrl-R`, `Mod+Z`/`Mod+Shift+Z`). */
export const DESKTOP_COMPOSER_FORMAT_KEYS: Readonly<Record<string, string>> = {
  bold: "MB",
  italic: "MI",
  code: "MX",
  link: "MU",
  bulletList: "MO",
};

/** Obsidian's direct formatting chords. Only formats whose chord has the same
 *  meaning everywhere get one; Mod+K stays the workspace prefix on macOS. */
export const DESKTOP_COMPOSER_FORMAT_CHORDS: Readonly<Record<string, string>> = {
  bold: "Mod+B",
  italic: "Mod+I",
};

/** The strokes of a leader path, for `DesktopCommand.sequence`. */
export function desktopLeaderSequence(path: string): string[] {
  return path === " " ? [DESKTOP_WORKSPACE_PREFIX, " "] : [
    DESKTOP_WORKSPACE_PREFIX,
    ...path,
  ];
}

export function desktopWorkspaceSequence(path: string): string {
  return desktopLeaderSequence(path).join(" → ");
}

export function desktopLeaderGroupSequence(group: string, key: string): string {
  return `${DESKTOP_WORKSPACE_PREFIX} → ${group} → ${key}`;
}

/** One source of truth for shortcut registration and every visible hint. */
export const DESKTOP_SHORTCUTS = {
  shortcuts: "Mod+/",
  commands: "Mod+Shift+P",
  stop: "Mod+.",
  saveDraft: "Mod+S",
  newSession: desktopWorkspaceSequence(DESKTOP_WORKSPACE_KEYS.newSession),
  settings: desktopWorkspaceSequence(DESKTOP_WORKSPACE_KEYS.settings),
  focusTopbar: desktopLeaderGroupSequence(
    DESKTOP_WORKSPACE_KEYS.focusTopbar,
    DESKTOP_WORKSPACE_KEYS.focusTopbar,
  ),
  focusSessions: desktopWorkspaceSequence(DESKTOP_WORKSPACE_KEYS.focusSessions),
  focusPrompt: desktopWorkspaceSequence(DESKTOP_WORKSPACE_KEYS.focusPrompt),
  focusConversation: desktopWorkspaceSequence(
    DESKTOP_WORKSPACE_KEYS.focusConversation,
  ),
  focusPlan: desktopWorkspaceSequence(DESKTOP_WORKSPACE_KEYS.focusPlan),
  focusQueue: desktopWorkspaceSequence(DESKTOP_WORKSPACE_KEYS.focusQueue),
  focusDrafts: desktopWorkspaceSequence(DESKTOP_WORKSPACE_KEYS.focusDrafts),
  cycleRegion: desktopWorkspaceSequence(DESKTOP_WORKSPACE_KEYS.cycleRegion),
  resize: desktopWorkspaceSequence(DESKTOP_WORKSPACE_KEYS.resize),
  toggleSourceMode: desktopWorkspaceSequence(
    DESKTOP_WORKSPACE_KEYS.toggleSourceMode,
  ),
  toggleSessions: desktopWorkspaceSequence(
    DESKTOP_WORKSPACE_KEYS.toggleSessions,
  ),
  togglePrompt: desktopWorkspaceSequence(DESKTOP_WORKSPACE_KEYS.togglePrompt),
  toggleConversation: desktopWorkspaceSequence(
    DESKTOP_WORKSPACE_KEYS.toggleConversation,
  ),
  switchSession: desktopWorkspaceSequence(DESKTOP_WORKSPACE_KEYS.switchSession),
  alternateSession: desktopWorkspaceSequence(
    DESKTOP_WORKSPACE_KEYS.alternateSession,
  ),
} as const;

export const DESKTOP_FOCUS_PROMPT_SHORTCUT = DESKTOP_SHORTCUTS.focusPrompt;
export const DESKTOP_FOCUS_PLAN_SHORTCUT = DESKTOP_SHORTCUTS.focusPlan;
export const DESKTOP_RESIZE_SELECT_SHORTCUT = DESKTOP_SHORTCUTS.resize;
export const DESKTOP_RESIZE_HINT = DESKTOP_SHORTCUTS.resize;

/** One stable meaning for every workspace-prefix continuation. */
export const DESKTOP_WORKSPACE_COMMANDS: Readonly<Record<string, string>> = {
  s: "workspace.focusSessions",
  p: "workspace.focusPrompt",
  // A group, not a command: `␣T` opens the Top bar layer (`␣TT` focuses it).
  t: "group:t",
  c: "workspace.focusConversation",
  l: "prompt.focusPlan",
  q: "prompt.focusQueue",
  d: "prompt.focusDrafts",
  n: "session.new",
  // Groups, not commands: their layers hold formatting, layout and toggles.
  m: "group:m",
  w: "group:w",
  u: "group:u",
  "/": "composer.slash",
  f: "composer.reference",
  a: "composer.attach",
  h: "composer.schedule",
  j: "composer.jumpFront",
  ",": "settings.open",
  " ": "session.switch",
  "`": "session.alternate",
  k: "commandPalette.open",
  // Scoped editors register their own `<id>.expand` under this one meaning.
  z: "editor.expand",
};

/** The leader key a command answers to, derived from its declared sequence. */
export function desktopLeaderKey(
  command: { sequence?: readonly string[] },
): string | null {
  const sequence = command.sequence ?? [];
  const [prefix, key] = sequence;
  return sequence.length === 2 && prefix === DESKTOP_WORKSPACE_PREFIX &&
      key !== undefined
    ? key.toLowerCase()
    : null;
}

/** `{ group, key }` for a grouped leader command (`␣T R`). */
export function desktopLeaderGroupKey(
  command: { sequence?: readonly string[] },
): { group: string; key: string } | null {
  const sequence = command.sequence ?? [];
  const [prefix, group, key] = sequence;
  return sequence.length === 3 && prefix === DESKTOP_WORKSPACE_PREFIX &&
      group !== undefined && key !== undefined
    ? { group: group.toLowerCase(), key: key.toLowerCase() }
    : null;
}

/** Space arms the leader only where Cowboy, not a text field, owns the key. */
export function isDesktopLeaderSpace(
  event: Pick<
    KeyboardEvent,
    "code" | "metaKey" | "ctrlKey" | "altKey" | "shiftKey" | "repeat"
  >,
): boolean {
  return event.code === "Space" && !event.metaKey && !event.ctrlKey &&
    !event.altKey && !event.shiftKey && !event.repeat;
}

type WorkspaceKeyEvent = Pick<
  KeyboardEvent,
  | "key"
  | "code"
  | "metaKey"
  | "ctrlKey"
  | "shiftKey"
  | "altKey"
  | "isComposing"
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

/** Resolve ownership before the generic IME-marker guard. A selected macOS
 * input source can label an otherwise ordinary physical shortcut key as
 * Process/229 while no marked-text transaction exists. The workspace sequence
 * may preempt those idle markers, but never a real browser/shared composition. */
export function desktopWorkspaceSequenceOwnsKey(
  event: WorkspaceKeyEvent,
  armed: boolean,
  sharedComposition: boolean,
  mac = isMac,
): boolean {
  if (sharedComposition || event.isComposing) return false;
  if (matchesDesktopWorkspacePrefix(event, mac)) return true;
  return armed && desktopWorkspaceContinuationKey(event, mac) !== null;
}
