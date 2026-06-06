import { useEffect, useSyncExternalStore } from "react";
import { DEFAULT_FONT_ID, getFontPreset } from "./fonts";

// Reading-comfort controls for the transcript: a font-size SCALE applied to the
// message/markdown content, and the transcript's horizontal PADDING (the side
// gutter). Both are persisted in localStorage and reactive across the app
// (Settings writes, Transcript reads) without prop-drilling — the same
// useSyncExternalStore pattern as vimSetting.
//
// Only the reading CONTENT takes the scale: it's applied as an `em` multiplier
// on the transcript scroll container, so the markdown body (which inherits) and
// its em-relative headings/code grow together, while MUI Typography chrome
// (tool-card captions, the app bar, the sidebar) keeps its fixed rem size. This
// mirrors liveview, where the reader column scales but the surrounding UI does
// not.

const FONT_KEY = "cowboy:font-scale";
const PAD_KEY = "cowboy:reading-pad";
const LINE_KEY = "cowboy:reading-line-height";
const VARIANT_KEY = "cowboy:font-variant";
const EVENT = "cowboy:reading-change";

// The reading font-family is one of the FONT_PRESETS in ./fonts (the full
// liveview set of self-hosted @fontsource faces). This store keeps only the
// selected id; the preset list + face loading live in ./fonts.

// Discrete presets — a dropdown taps cleanly on touch where a slider is fiddly
// and a number field pops the iOS keyboard (liveview's reasoning). The scale is
// a multiplier on the inherited reading size (1 = unchanged); padding is px.
export const FONT_SCALE_PRESETS: number[] = [
  0.55, 0.6, 0.65, 0.7, 0.75, 0.8, 0.85, 0.9, 1, 1.1, 1.25,
];
export const PADDING_PRESETS: number[] = [8, 12, 16, 20, 24, 32, 48];
// Line-height of the reading prose (paragraphs/lists). Headings + code keep
// their own fixed leading; this drives the body text's line spacing.
export const LINE_HEIGHT_PRESETS: number[] = [1.3, 1.4, 1.5, 1.6, 1.8, 2];

export const FONT_SCALE_DEFAULT = 0.85;
// Default reading comfort: 16px side gutter, 1.8 line-height, 0.85 scale — the
// product-chosen defaults for an unset/garbage value (a user's own picks still
// win). Source Serif 4 is the default face (see fonts.ts DEFAULT_FONT_ID).
export const PADDING_DEFAULT = 16;
export const LINE_HEIGHT_DEFAULT = 1.8;

export interface ReadingSettings {
  /** Multiplier applied to the transcript's reading font size (1 = unchanged). */
  fontScale: number;
  /** Horizontal padding (px) of the transcript scroll column. */
  padding: number;
  /** Line-height multiplier applied to the reading prose (1.5 = unchanged). */
  lineHeight: number;
  /** Id of the reading font-family variant (see FONT_VARIANTS). */
  fontVariant: string;
}

// Snap a stored value to the nearest preset so the dropdown always shows a
// selected option even if an older/clamped value falls between presets.
export function nearestPreset(value: number, presets: number[]): number {
  return presets.reduce(
    (a, b) => (Math.abs(b - value) < Math.abs(a - value) ? b : a),
    presets[0] ?? value,
  );
}

function clampToRange(n: number, min: number, max: number, def: number): number {
  return Number.isFinite(n) && n >= min && n <= max ? n : def;
}

function read(): ReadingSettings {
  const fs = Number(globalThis.localStorage?.getItem(FONT_KEY));
  const pad = Number(globalThis.localStorage?.getItem(PAD_KEY));
  const lh = Number(globalThis.localStorage?.getItem(LINE_KEY));
  const variant = globalThis.localStorage?.getItem(VARIANT_KEY) ?? "";
  return {
    // Bounds are wider than the preset band so a hand-edited value is honoured
    // but a garbage/missing one snaps back to the default.
    fontScale: clampToRange(fs, 0.5, 2, FONT_SCALE_DEFAULT),
    padding: clampToRange(pad, 0, 96, PADDING_DEFAULT),
    lineHeight: clampToRange(lh, 1, 2.5, LINE_HEIGHT_DEFAULT),
    // Unknown/missing id resolves to the default preset.
    fontVariant: getFontPreset(variant).id,
  };
}

// useSyncExternalStore requires getSnapshot to return a STABLE reference while
// nothing changed, or React re-renders forever. Cache the last snapshot and
// rebuild it only when a write (or another tab's `storage` event) bumps a value.
let snapshot: ReadingSettings = read();

function refresh(): void {
  snapshot = read();
}

function subscribe(onChange: () => void): () => void {
  const handler = (): void => {
    refresh();
    onChange();
  };
  globalThis.addEventListener?.(EVENT, handler);
  globalThis.addEventListener?.("storage", handler); // other tabs
  return () => {
    globalThis.removeEventListener?.(EVENT, handler);
    globalThis.removeEventListener?.("storage", handler);
  };
}

// Stable default for the SSR/initial path (getServerSnapshot must be referential
// -ly stable too).
const SERVER_SNAPSHOT: ReadingSettings = {
  fontScale: FONT_SCALE_DEFAULT,
  padding: PADDING_DEFAULT,
  lineHeight: LINE_HEIGHT_DEFAULT,
  fontVariant: DEFAULT_FONT_ID,
};

export function useReadingSettings(): ReadingSettings {
  return useSyncExternalStore(
    subscribe,
    () => snapshot,
    () => SERVER_SNAPSHOT,
  );
}

export function setFontScale(scale: number): void {
  globalThis.localStorage?.setItem(FONT_KEY, String(scale));
  // dispatchEvent runs listeners synchronously, so `snapshot` is refreshed
  // before this returns — the caller's next render sees the new value.
  globalThis.dispatchEvent?.(new Event(EVENT));
}

export function setPadding(px: number): void {
  globalThis.localStorage?.setItem(PAD_KEY, String(px));
  globalThis.dispatchEvent?.(new Event(EVENT));
}

export function setLineHeight(value: number): void {
  globalThis.localStorage?.setItem(LINE_KEY, String(value));
  globalThis.dispatchEvent?.(new Event(EVENT));
}

export function setFontVariant(id: string): void {
  globalThis.localStorage?.setItem(VARIANT_KEY, id);
  globalThis.dispatchEvent?.(new Event(EVENT));
}

/**
 * Apply the reading settings GLOBALLY: scale the root <html> font-size (so every
 * rem/em surface — MUI Typography, the transcript prose, AND the composer —
 * tracks the font-size setting), and publish the reading line-height as a CSS
 * variable. The composer (CodeMirror theme + native textarea) reads that var to
 * match the prose leading, since neither can read the React settings store. Both
 * now track the setting fully, up and down. px-based layout (MUI spacing,
 * safe-area insets) holds. Mount once at the app root (see main.tsx).
 */
export function useGlobalFontScale(): void {
  const { fontScale, lineHeight } = useReadingSettings();
  useEffect(() => {
    const root = globalThis.document?.documentElement;
    if (!root) return;
    root.style.fontSize = `${String(fontScale * 100)}%`;
    root.style.setProperty("--cowboy-reading-line-height", String(lineHeight));
  }, [fontScale, lineHeight]);
}

/**
 * Lazily inject the @fontsource faces for the currently selected reading font,
 * and expose the resolved family stack via the `--cowboy-reading-font` CSS
 * variable. Mount once at the app root (see main.tsx). The selected face's
 * woff2 is fetched only when first chosen; the default "System" preset loads
 * nothing. Mirrors liveview's useFont.
 */
export function useReadingFontFaces(): void {
  const { fontVariant } = useReadingSettings();
  useEffect(() => {
    const preset = getFontPreset(fontVariant);
    globalThis.document?.documentElement.style.setProperty(
      "--cowboy-reading-font",
      preset.stack,
    );
    void preset.load();
  }, [fontVariant]);
}
