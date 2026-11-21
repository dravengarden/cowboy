import { alpha, type Theme } from "@mui/material/styles";

// Paint only: no scroll listener, backdrop filter or promoted child layer.
// The host owns the safe-area inset; embedded readers default to zero.
export function readerTopInset(inset?: string): string {
  return `var(--cowboy-reader-top-inset, ${inset ?? "0px"})`;
}

export function readerTopEdge(theme: Theme, inset?: string) {
  const top = readerTopInset(inset);
  const color = theme.palette.background.default;
  return {
    "&::after": {
      content: '""', position: "absolute", top: 0, left: 0, right: 0,
      height: `calc(${top} + 20px)`, zIndex: 2, pointerEvents: "none",
      background: `linear-gradient(to bottom, ${color} 0px, ${alpha(color, 0.94)} calc(${top} * 0.55), ${alpha(color, 0.55)} ${top}, ${alpha(color, 0)} 100%)`,
    },
  };
}

/** Distance below the status area over which the blur fades out. A reader at
 *  rest keeps its first item at least this far below the inset, so only
 *  scrolled content is ever blurred. */
export const READER_TOP_FROST_FADE = "16px";
// The same lens as the Agent status strip (`frostedStatusChrome`): quiet, so
// scrolled text stays a silhouette behind the system icons.
const FROST_BLUR = "blur(24px) saturate(150%)";

/** Frosted top edge for a full-bleed cover reader that owns its own scroller:
 *  content passes behind the status area and blurs instead of being cut off.
 *  Apply it to the non-scrolling positioned parent, never to the scroller (a
 *  scroller's mask would clip it) and never to a swipe-peek surface: unlike
 *  `readerTopEdge` this layer owns a backdrop filter. It is one bounded layer,
 *  the mirror image of `readerBottomEdge`. The reader keeps its first item
 *  clear of it with its own top padding of `readerTopInset()` plus
 *  `READER_TOP_FROST_FADE`.
 */
export function readerTopFrost(theme: Theme, inset?: string) {
  const top = readerTopInset(inset);
  const color = theme.palette.background.default;
  const mask = `linear-gradient(to bottom, #000 calc(100% - ${READER_TOP_FROST_FADE}), transparent)`;
  return {
    "&::after": {
      content: '""', position: "absolute", top: 0, left: 0, right: 0,
      height: `calc(${top} + ${READER_TOP_FROST_FADE})`, zIndex: 2, pointerEvents: "none",
      backdropFilter: FROST_BLUR, WebkitBackdropFilter: FROST_BLUR,
      maskImage: mask, WebkitMaskImage: mask,
      background: `linear-gradient(to bottom, ${alpha(color, 0.9)} 0px, ${alpha(color, 0.72)} ${top}, ${alpha(color, 0.2)} 100%)`,
      "@media (prefers-reduced-transparency: reduce)": {
        backdropFilter: "none", WebkitBackdropFilter: "none",
        background: `linear-gradient(to bottom, ${color} ${top}, ${alpha(color, 0)} 100%)`,
      },
    },
  };
}
