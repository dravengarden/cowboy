// Desktop modifier-key facts, computed once at load. The composer's keyboard
// scheme needs two distinct desktop modifiers — a PRIMARY (send) and a SECONDARY
// (save-as-draft) — and they differ by OS:
//   - macOS:  primary = ⌘ (Meta), secondary = ⌃ (Control)
//   - other:  primary = Ctrl,     secondary = Alt
//     (Ctrl is the primary there, so the draft chord falls back to Alt.)
// Same detection as theme.ts's safe-area guess, kept here so non-theme code
// (ComposerEditor keymap, the hint chips) can share it.
function detectMac(): boolean {
  const nav = globalThis.navigator as
    | (Navigator & { userAgentData?: { platform?: string } })
    | undefined;
  const ua = nav?.userAgent ?? "";
  const platform = nav?.userAgentData?.platform ?? nav?.platform ?? "";
  return /mac/i.test(platform) || /Macintosh/i.test(ua);
}

export const isMac: boolean = detectMac();

/** Display label for the primary (send) modifier. */
export const MOD_LABEL: string = isMac ? "⌘" : "Ctrl";
/** Display label for the physical Alt/Option modifier used by Desktop commands. */
export const ALT_LABEL: string = isMac ? "⌥" : "Alt";
/** Display prefix for Cowboy's collision-resistant command namespace. */
export const COWBOY_MOD_LABEL: string = isMac ? "⌘⌥⇧" : "Ctrl+Alt+Shift+";
/** Display label for the secondary (save-as-draft) modifier. */
export const DRAFT_LABEL: string = isMac ? "⌃" : "Alt";
/** Return key glyph used across the hint chips. */
export const ENTER_LABEL = "⏎";

// Structural so both a native KeyboardEvent and a React.KeyboardEvent qualify.
type Mods = {
  readonly metaKey: boolean;
  readonly ctrlKey: boolean;
  readonly altKey: boolean;
};

/**
 * True when this key event carries the PRIMARY (send) modifier and no
 * conflicting one. On mac that's ⌘ without Ctrl; elsewhere Ctrl without Alt.
 */
export function hasSendMod(e: Mods): boolean {
  return isMac ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.altKey;
}

/**
 * True when this key event carries the SECONDARY (draft) modifier. On mac that's
 * ⌃ without ⌘; elsewhere Alt without Ctrl.
 */
export function hasDraftMod(e: Mods): boolean {
  return isMac ? e.ctrlKey && !e.metaKey : e.altKey && !e.ctrlKey;
}
