export type TranscriptRowContainment = "layout paint" | "layout" | "none";

/**
 * Settled mobile rows keep their independent paint boundary so an adjacent
 * streamed row cannot make iOS WebKit briefly reuse an old tool-card layer.
 * The row that is actively growing must remain in the scroller's paint flow,
 * though: inside a column-reverse scroller, paint containment can otherwise
 * rasterize its new height one frame after scroll anchoring has positioned it,
 * which looks like the message itself is shaking even while its layout box
 * stays still.
 *
 * A running tool card belongs here too. It is a new row whose pulse makes it a
 * compositor layer, and iOS WebKit can leave that layer unpainted inside a
 * fresh paint boundary: the row keeps its height and the card is missing.
 *
 * A settled row with lazy media cannot own a paint boundary either. iOS
 * WebKit may rasterize that row while the image is still off-screen, then keep
 * the empty layer after decode when a column-reverse scroller reveals it. Keep
 * layout containment for stable row geometry, but let the scroller paint the
 * image and the rest of its user bubble.
 *
 * On Desktop, keep every row at constant `contain: "layout"` (both while
 * streaming/running and once settled) instead of toggling between `"none"` and
 * `"layout paint"`. On macOS WebKit (verified on macOS 26.6.2 / WebKit
 * 21624.5.1.11.3), mutating a row's containment across the stacking-context
 * boundary inside the `column-reverse` scroller causes WebKit to tear down
 * `"scrolled contents"` and leak its tiled `WKCompositingLayer` and
 * `<CAIOSurface>` tree in an unreachable QuartzCore retain cycle on every
 * settled turn, eventually exhausting macOS's global IOSurface limit. Constant
 * `"layout"` keeps every Desktop row a stable stacking context without adding
 * a paint boundary (avoiding paint-boundary jitter/blank holes and the
 * `contain: paint` short-to-overflowing leak) while leaving Mobile's iOS
 * WebKit policy unchanged.
 */
export function transcriptRowContainment(
  streaming: boolean,
  lazyMedia = false,
  desktop = false,
): TranscriptRowContainment {
  if (desktop) return "layout";
  if (streaming) return "none";
  return lazyMedia ? "layout" : "layout paint";
}

export type TranscriptScrollerChildSx = {
  readonly flex: "0 0 auto";
  readonly contain?: "layout";
};

/**
 * Every direct child of the Desktop transcript scroller — not only `[data-key]`
 * message/tool rows, but also sibling chrome such as the trailing activity row
 * (`ThinkingIndicator`), `ScrollbackLoadingSkeleton`, optimistic user bubbles,
 * compaction rows, and skeletons — must stay at constant `contain: "layout"`.
 *
 * On macOS 26.6.2 WebKit (21624.5.1.11.3), mounting, unmounting, or animating a
 * composited layer (for example `CircularProgress`, `Skeleton`, or `maskImage`)
 * inside a non-contained direct child of the `column-reverse` scroller still
 * forces WebKit to tear down `"scrolled contents"` and leak its tiled
 * `WKCompositingLayer` and `<CAIOSurface>` tree into a QuartzCore retain cycle.
 * Mobile stays untouched (`flex: "0 0 auto"` only) so iOS WebKit's per-row
 * paint containment and drawer overrides keep their existing behavior.
 */
export function transcriptScrollerChildSx(
  desktop: boolean,
): TranscriptScrollerChildSx {
  return desktop
    ? { flex: "0 0 auto", contain: "layout" }
    : { flex: "0 0 auto" };
}
