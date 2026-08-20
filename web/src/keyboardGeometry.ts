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
/** Overlaps below this are browser chrome, not a software keyboard.
 *  iOS keyboards are ~260–340px; Safari's compact URL pill + form
 *  accessory land in the 44–110px range. */
export const keyboardOpenMinOverlapPx = 120;

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
  return visualOverlap > keyboardOpenMinOverlapPx ||
    (geometry.editableFocused && resizedOverlap > keyboardOpenMinOverlapPx);
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

/** Layout box that `position:fixed` actually uses. iOS Safari tabs keep
 *  `html.clientHeight` on the pre-keyboard page while `innerHeight` tracks
 *  the visual viewport — min() with innerHeight (paintedLayoutHeight) would
 *  clamp offsetTop to 0 and leave a cover sheet in the wrong slice. */
export function fixedLayoutHeight(
  clientHeight: number,
  rootHeight = 0,
  innerHeight = 0,
): number {
  const layout = [clientHeight, rootHeight].filter((height) => height > 0);
  const tallest = layout.length > 0 ? Math.max(...layout) : 0;
  return tallest > 0 ? tallest : innerHeight;
}

/** Pin a full-bleed cover to the visual viewport. Safari pans offsetTop
 *  over a taller layout while the keyboard is up; a 100dvh New Session
 *  cover then shows its empty body + footer and hides Title. */
export function visualViewportBox(
  layoutHeight: number,
  visualHeight: number,
  visualOffsetTop = 0,
): { offset: number; height: number } {
  const height = Math.round(Math.max(0, visualHeight));
  const layout = Math.round(Math.max(0, layoutHeight));
  const maxOffset = Math.max(0, layout - height);
  const offset = Math.round(
    Math.max(0, Math.min(visualOffsetTop, maxOffset)),
  );
  return { offset, height };
}

/** iOS Safari can move `window.innerHeight` independently from the CSS boxes
 *  that actually contain the app. Usually it stays at the pre-keyboard height
 *  after `html` / `#root` have shortened; after dismissing a portaled cover it
 *  can do the inverse and shrink while both DOM boxes stay tall. In either
 *  direction the DOM boxes own the painted layout. `innerHeight` is only a
 *  fallback when neither box has a usable measurement. */
export function paintedLayoutHeight(
  innerHeight: number,
  clientHeight: number,
  rootHeight = 0,
): number {
  const painted = [clientHeight, rootHeight].filter((height) => height > 0);
  const smallestPainted = painted.length > 0 ? Math.min(...painted) : 0;
  return smallestPainted > 0 ? smallestPainted : innerHeight;
}

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

/** How much of the painted page still extends *below* the visual viewport.
 *
 *  `paintedHeight - visualHeight` over-counts when offsetTop > 0: Safari
 *  pans the visual viewport to keep the focused field on screen, so part
 *  of that difference is already above the window.
 *
 *  After `interactive-widget=resizes-content` the painted box has already
 *  excluded the keyboard. Safari tabs then report a *further* shortfall
 *  equal to the compact URL pill (iOS 26) sitting *outside* that box —
 *  `offsetTop` is often 0, so subtracting it does nothing. Padding that
 *  chrome remainder (typically 50–110px) is the lavender band between the
 *  composer and `cowboy.stormbird.xyz`. PWA has no pill (remainder ≈ 0);
 *  the native shell never publishes `--kb-inset`. Remainders at or below
 *  `keyboardOpenMinOverlapPx` are chrome, not cover. */
export function keyboardCoverOverlap(
  paintedHeight: number,
  visualHeight: number,
  visualOffsetTop = 0,
): number {
  if (isUnreliableVisualViewport(paintedHeight, visualHeight)) return 0;
  const covered = Math.max(0, paintedHeight - visualHeight);
  const offset = Math.max(0, Math.min(visualOffsetTop, covered));
  const below = Math.round(covered - offset);
  if (below <= keyboardOpenMinOverlapPx) return 0;
  return clampKeyboardOverlap(below, paintedHeight);
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
