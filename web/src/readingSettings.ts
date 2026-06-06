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
const VARIANT_KEY = "cowboy:font-variant";
const EVENT = "cowboy:reading-change";

// The reading font-family is one of the FONT_PRESETS in ./fonts (the full
// liveview set of self-hosted @fontsource faces). This store keeps only the
// selected id; the preset list + face loading live in ./fonts.

// Discrete presets — a dropdown taps cleanly on touch where a slider is fiddly
// and a number field pops the iOS keyboard (liveview's reasoning). The scale is
// a multiplier on the inherited reading size (1 = unchanged); padding is px.
export const FONT_SCALE_PRESETS: number[] = [0.85, 0.9, 1, 1.1, 1.25, 1.4, 1.6];
export const PADDING_PRESETS: number[] = [0, 8, 16, 24, 32, 48];

export const FONT_SCALE_DEFAULT = 1;
// 24px default gutter — a comfortable, breakpoint-independent side margin for
// the reading column (one preset above the old 16px). Users can still pick any
// PADDING_PRESETS value (down to None); this is only the unset/garbage fallback.
export const PADDING_DEFAULT = 24;

export interface ReadingSettings {
  /** Multiplier applied to the transcript's reading font size (1 = unchanged). */
  fontScale: number;
  /** Horizontal padding (px) of the transcript scroll column. */
  padding: number;
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
  const variant = globalThis.localStorage?.getItem(VARIANT_KEY) ?? "";
  return {
    // Bounds are wider than the preset band so a hand-edited value is honoured
    // but a garbage/missing one snaps back to the default.
    fontScale: clampToRange(fs, 0.6, 2, FONT_SCALE_DEFAULT),
    padding: clampToRange(pad, 0, 96, PADDING_DEFAULT),
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

export function setFontVariant(id: string): void {
  globalThis.localStorage?.setItem(VARIANT_KEY, id);
  globalThis.dispatchEvent?.(new Event(EVENT));
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
