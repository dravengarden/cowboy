export type TranscriptRowContainment = "layout paint" | "layout" | "none";

/**
 * Settled rows keep their independent paint boundary so an adjacent streamed
 * row cannot make WebKit briefly reuse an old tool-card layer. The row that is
 * actively growing must remain in the scroller's paint flow, though: inside a
 * column-reverse scroller, paint containment can otherwise rasterize its new
 * height one frame after scroll anchoring has positioned it, which looks like
 * the message itself is shaking even while its layout box stays still.
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
 */
export function transcriptRowContainment(
  streaming: boolean,
  lazyMedia = false,
): TranscriptRowContainment {
  if (streaming) return "none";
  return lazyMedia ? "layout" : "layout paint";
}
