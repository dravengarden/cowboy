import { useEffect, useLayoutEffect } from "react";
import { DEFAULT_FONT_ID, getFontPreset } from "./fonts";
import { persisted, useStore } from "./_store/mod.ts";

// Application readability controls: font-size SCALE applies at the root so
// transcript prose, MUI chrome, editors, keycaps, and functional icons move
// together; PADDING controls only the transcript's horizontal gutter. Both are
// persisted in localStorage and reactive across the app (Settings writes,
// consumers read) without prop-drilling — the same useSyncExternalStore pattern
// as vimSetting.

const FONT_KEY = "cowboy:font-scale";
const PAD_KEY = "cowboy:reading-pad";
const LINE_KEY = "cowboy:reading-line-height";
const VARIANT_KEY = "cowboy:font-variant";

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

export const FONT_SCALE_DEFAULT = 0.55;
// Default reading comfort: 8px side gutter, 1.6 line-height, 0.55 scale — the
// product-chosen defaults for an unset/garbage value (a user's own picks still
// win). The system stack is the default face (see fonts.ts DEFAULT_FONT_ID).
export const PADDING_DEFAULT = 8;
export const LINE_HEIGHT_DEFAULT = 1.6;

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

// A numeric reading pref, stored as `String(n)` (legacy format preserved). A
// missing/empty/garbage/out-of-range value snaps back to `def` — crucially a
// MISSING key must use `def`, NOT coerce to 0 (for padding min 0, the old
// `Number(null) === 0` silently overrode the default — the "first launch shows
// 8px in Settings but 0px gutter" bug). Bounds are wider than the preset band so
// a hand-edited value is honoured.
function numPref(key: string, min: number, max: number, def: number) {
  return persisted(key, def, {
    serialize: String,
    deserialize: (raw) => {
      if (raw === "") return def;
      const n = Number(raw);
      return Number.isFinite(n) && n >= min && n <= max ? n : def;
    },
  });
}

const fontScaleStore = numPref(FONT_KEY, 0.5, 2, FONT_SCALE_DEFAULT);
const paddingStore = numPref(PAD_KEY, 0, 96, PADDING_DEFAULT);
const lineHeightStore = numPref(LINE_KEY, 1, 2.5, LINE_HEIGHT_DEFAULT);
// The variant store keeps the raw id but resolves unknown/missing ids to the
// default preset on read (so the dropdown always has a valid selection).
const fontVariantStore = persisted(VARIANT_KEY, DEFAULT_FONT_ID, {
  serialize: (id) => id,
  deserialize: (raw) => getFontPreset(raw).id,
});

// Combine the four independent stores. The returned object is fresh per render
// (fine — it's a hook return, not a getSnapshot), and consumers destructure to
// stable primitives; a component re-renders only when a store it reads changes.
export function useReadingSettings(): ReadingSettings {
  return {
    fontScale: useStore(fontScaleStore),
    padding: useStore(paddingStore),
    lineHeight: useStore(lineHeightStore),
    fontVariant: useStore(fontVariantStore),
  };
}

export function setFontScale(scale: number): void {
  fontScaleStore.set(scale);
}

export function setPadding(px: number): void {
  paddingStore.set(px);
}

export function setLineHeight(value: number): void {
  lineHeightStore.set(value);
}

export function setFontVariant(id: string): void {
  fontVariantStore.set(id);
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
  // This controls the geometry of the entire app. Apply it before paint so the
  // first visible frame does not render at 100% and then jump to the persisted
  // scale (a sizeable CLS on both Desktop and Mobile).
  useLayoutEffect(() => {
    const root = globalThis.document?.documentElement;
    if (!root) return;
    root.style.fontSize = `${String(fontScale * 100)}%`;
    root.style.setProperty("--cowboy-font-scale", String(fontScale));
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
