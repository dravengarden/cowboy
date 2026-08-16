/** Native vertical momentum for Agent/Code scrollports.
 *  `pan-y` lets iOS start async scrolling without waiting for the shell's
 *  non-passive horizontal recognizer. Horizontal swipes still reach the
 *  drawer/pager after the existing direction lock. */
export const mobileNativeYScrollSx = {
  overflowY: "auto",
  overscrollBehaviorY: "contain",
  touchAction: "pan-y pinch-zoom",
  WebkitOverflowScrolling: "touch",
} as const;
