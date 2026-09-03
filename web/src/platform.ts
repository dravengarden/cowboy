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

/** Document root class that owns touch hover/focus paint, not a live media query. */
export const COARSE_POINTER_ROOT_CLASS = "cowboy-coarse-pointer";

/**
 * Document root class for the narrow scroll-edge material immediately below
 * iPhone standalone status chrome. `navigator.standalone` deliberately keeps
 * this out of browser tabs, Android, and the native WKWebView shell; the
 * physical screen's short side distinguishes an iPhone from an iPad even when
 * the latter is running Cowboy in a narrow split-view window.
 */
export const PHONE_STANDALONE_ROOT_CLASS = "cowboy-phone-standalone";

export interface PhoneStandaloneInput {
  readonly appleStandalone: boolean;
  readonly coarsePointer: boolean;
  readonly screenWidth: number;
  readonly screenHeight: number;
}

export function prefersCoarsePointer(
  input: {
    readonly maxTouchPoints?: number;
    readonly anyPointerCoarse?: boolean;
  } = {
    maxTouchPoints: globalThis.navigator?.maxTouchPoints,
    anyPointerCoarse: globalThis.matchMedia?.("(any-pointer: coarse)").matches ??
      false,
  },
): boolean {
  return (input.maxTouchPoints ?? 0) > 0 || input.anyPointerCoarse === true;
}

export function syncCoarsePointerRootClass(
  doc: {
    readonly documentElement?: {
      readonly classList: { toggle(name: string, force: boolean): boolean };
    };
  } | null = globalThis.document,
): boolean {
  const coarse = prefersCoarsePointer();
  doc?.documentElement?.classList.toggle(COARSE_POINTER_ROOT_CLASS, coarse);
  return coarse;
}

export function prefersPhoneStandaloneStatusShelf(
  input: PhoneStandaloneInput = {
    appleStandalone: (globalThis.navigator as
      | (Navigator & { standalone?: boolean })
      | undefined)
      ?.standalone === true,
    coarsePointer: prefersCoarsePointer(),
    screenWidth: globalThis.screen?.width ?? Number.POSITIVE_INFINITY,
    screenHeight: globalThis.screen?.height ?? Number.POSITIVE_INFINITY,
  },
): boolean {
  const shortSide = Math.min(input.screenWidth, input.screenHeight);
  return input.appleStandalone && input.coarsePointer &&
    Number.isFinite(shortSide) && shortSide > 0 && shortSide < 700;
}

export function syncPhoneStandaloneRootClass(
  doc: {
    readonly documentElement?: {
      readonly classList: { toggle(name: string, force: boolean): boolean };
    };
  } | null = globalThis.document,
  input?: PhoneStandaloneInput,
): boolean {
  const phoneStandalone = prefersPhoneStandaloneStatusShelf(input);
  doc?.documentElement?.classList.toggle(
    PHONE_STANDALONE_ROOT_CLASS,
    phoneStandalone,
  );
  return phoneStandalone;
}
