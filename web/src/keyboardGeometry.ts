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
