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
export function clampKeyboardOverlap(
  overlap: number,
  layoutHeight: number,
): number {
  if (overlap <= 0 || layoutHeight <= 0) return 0;
  const max = Math.round(layoutHeight * 0.52);
  return Math.min(overlap, max);
}
