export type TranscriptRowContainment = "layout paint" | "none";

/**
 * Settled rows keep their independent paint boundary so an adjacent streamed
 * row cannot make WebKit briefly reuse an old tool-card layer. The row that is
 * actively growing must remain in the scroller's paint flow, though: inside a
 * column-reverse scroller, paint containment can otherwise rasterize its new
 * height one frame after scroll anchoring has positioned it, which looks like
 * the message itself is shaking even while its layout box stays still.
 */
export function transcriptRowContainment(
  streaming: boolean,
): TranscriptRowContainment {
  return streaming ? "none" : "layout paint";
}
