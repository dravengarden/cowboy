export interface KeyboardGeometry {
  layoutHeight: number;
  visualHeight: number;
  baselineHeight: number;
  editableFocused: boolean;
}

// Some iOS third-party keyboards resize the WKWebView's layout and visual
// viewports together. In that mode layoutHeight - visualHeight is zero even
// though several hundred pixels are occupied by the keyboard. Keep the last
// keyboard-free height as a second signal, but only trust it while an editor is
// focused so rotation/browser chrome changes cannot masquerade as a keyboard.
export function inferKeyboardOpen(geometry: KeyboardGeometry): boolean {
  const visibleHeight = Math.min(
    geometry.layoutHeight,
    geometry.visualHeight,
  );
  const visualOverlap = Math.max(
    0,
    geometry.layoutHeight - geometry.visualHeight,
  );
  const resizedOverlap = Math.max(
    0,
    geometry.baselineHeight - visibleHeight,
  );
  return visualOverlap > 120 ||
    (geometry.editableFocused && resizedOverlap > 120);
}

/** iOS can report visualViewport.height ≈ 0 for one keyboard frame.
 *  Publishing that as --kb-inset pads the column off-screen and flashes. */
export function isUnreliableVisualViewport(
  layoutHeight: number,
  visualHeight: number,
): boolean {
  if (!(layoutHeight > 0) || !(visualHeight >= 0)) return true;
  if (visualHeight < 80) return true;
  return layoutHeight - visualHeight > layoutHeight * 0.55;
}

export function clampKeyboardOverlap(
  overlap: number,
  layoutHeight: number,
): number {
  if (overlap <= 0 || layoutHeight <= 0) return 0;
  if (isUnreliableVisualViewport(layoutHeight, layoutHeight - overlap)) {
    return 0;
  }
  const max = Math.round(layoutHeight * 0.52);
  return Math.min(overlap, max);
}

/** iOS Safari often keeps `window.innerHeight` on the pre-keyboard layout
 *  viewport after `interactive-widget=resizes-content` (or Safari chrome)
 *  has already shortened the painted `html` / `#root` box. Padding must
 *  follow the painted box. Using the stale innerHeight double-lifts the
 *  composer above chrome that is already outside the webview. */
export function paintedLayoutHeight(
  innerHeight: number,
  clientHeight: number,
  rootHeight = 0,
): number {
  const painted = [clientHeight, rootHeight].filter((height) => height > 0);
  const smallestPainted = painted.length > 0 ? Math.min(...painted) : 0;
  if (innerHeight > 0 && smallestPainted > 0) {
    return Math.min(innerHeight, smallestPainted);
  }
  return innerHeight > 0 ? innerHeight : smallestPainted;
}

/** Sub-pixel / chrome jitter that still means the layout already fits. */
export const keyboardFittedEpsilonPx = 8;

/** URL-bar / status-bar jitter allowed when learning a new rest height.
 *  A larger drop is a software keyboard, not a new orientation baseline. */
export const keyboardFreeBaselineSlackPx = 80;

/** Expand → collapse remounts the editor and briefly loses focus. If we
 *  adopt the keyboard-sized visual height as the new rest baseline in that
 *  gap, later focused frames see no overlap and the compact card + session
 *  nav come back over a still-visible keyboard. */
export function shouldLearnKeyboardFreeBaseline(
  baselineHeight: number,
  visibleHeight: number,
): boolean {
  if (!(baselineHeight > 0) || !(visibleHeight > 0)) return true;
  return visibleHeight >= baselineHeight - keyboardFreeBaselineSlackPx;
}

/** How much of the painted page still extends below the visual viewport.
 *  Do not add visualViewport.offsetTop: rubber-band pans inflate it and
 *  lift the composer too high. */
export function keyboardCoverOverlap(
  paintedHeight: number,
  visualHeight: number,
): number {
  if (isUnreliableVisualViewport(paintedHeight, visualHeight)) return 0;
  const raw = Math.round(Math.max(0, paintedHeight - visualHeight));
  if (raw <= keyboardFittedEpsilonPx) return 0;
  return clampKeyboardOverlap(raw, paintedHeight);
}

/** iOS Safari / installed PWA form accessory (∧ ∨ ✓). The native shell
 *  strips this bar. On PWA it sits *below* the visual viewport after
 *  resizes-content, so it must not be folded into `--kb-inset` or the
 *  composer grows an empty band. Cover sheets (New Session) keep Title
 *  above it without padding the whole app. */
export const iosPwaKeyboardAccessoryPx = 44;

export function isAppleTouchDevice(input: {
  readonly userAgent?: string;
  readonly platform?: string;
  readonly maxTouchPoints?: number;
}): boolean {
  const userAgent = input.userAgent ?? "";
  const platform = input.platform ?? "";
  return /iP(hone|ad|od)/.test(userAgent) ||
    (platform === "MacIntel" && (input.maxTouchPoints ?? 0) > 1);
}

export function pwaKeyboardAccessoryOverlap(input: {
  readonly nativeShell: boolean;
  readonly appleTouch: boolean;
  readonly editableFocused: boolean;
}): number {
  if (input.nativeShell || !input.appleTouch || !input.editableFocused) {
    return 0;
  }
  return iosPwaKeyboardAccessoryPx;
}

export function publishedKeyboardInset(
  coverOverlap: number,
  accessoryOverlap: number,
): number {
  return coverOverlap + accessoryOverlap;
}
