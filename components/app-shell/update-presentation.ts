// How a pending update looks while it arrives.
//
// The bar that announces a deploy is also the control that takes it, and it is
// its own progress bar: one element whose background is a hard-stopped gradient
// sized to the downloaded share (docs/offline-first-sync.md §Update policy).
// Kept pure and DOM-free next to `update-policy`, which owns when the swap
// happens; this owns only what the user sees while it is coming.

import type { UpdatePhase } from "./update-policy.ts";

/** How much of the bar is filled, 0-1.
 *
 *  A stalled download keeps the ground it took — a bar that rewound would read
 *  as the update giving up — and a page no service worker controls has no
 *  progress to show at all, so it fills only once it is ready to press. */
export function updateFillShare(phase: UpdatePhase, progress: number | undefined): number {
  // A rollback notice is not a progress bar that stopped somewhere; it is a
  // statement, and a statement is whole.
  if (phase === "ready" || phase === "reloading" || phase === "rejected") return 1;
  return progress ?? 0;
}

/** The downloaded share as a whole percent. Held under 100 until the bits are
 *  really here: a bar reading 100% while the label still says it is
 *  downloading teaches the user to distrust both. */
export function updatePercentLabel(
  progress: number | undefined,
  settled: boolean,
): string | undefined {
  if (progress === undefined) return undefined;
  const percent = Math.round(Math.max(0, Math.min(1, progress)) * 100);
  return `${String(settled ? percent : Math.min(99, percent))}%`;
}

/** The bar's background, as one paint-only fill over a darker track.
 *
 *  No extra node, no transform, no shadow — the phone's moving chrome forbids
 *  all three (docs/mobile-spatial-presentation.md §2.1) — and the full state is
 *  simply the bar's ordinary solid colour, so arriving needs no separate
 *  celebration and the control never changes shape under the thumb.
 *
 *  `animate` must be false unless a batch was really seen to land mid-download:
 *  a build that was already cached resolves at once and has to snap. Sweeping a
 *  bar through progress it never spent is the kind of small lie that costs an
 *  offline-first app its credibility. */
export function updateFillSx(
  fill: string,
  track: string,
  share: number,
  animate: boolean,
): Record<string, string> {
  const percent = Math.round(Math.max(0, Math.min(1, share)) * 100);
  return {
    backgroundColor: track,
    backgroundImage: `linear-gradient(0deg, ${fill}, ${fill})`,
    backgroundRepeat: "no-repeat",
    backgroundSize: `${String(percent)}% 100%`,
    // The compositor settle curve, over the batch-granular jumps the worker
    // reports.
    transition: animate ? "background-size 400ms cubic-bezier(0.32, 0.72, 0, 1)" : "none",
  };
}

/** Whether the update presents as a hairline rather than as the full bar.
 *
 *  A download nobody asked for is not news. It is also not actionable: the
 *  bits arrive at the speed of the network, and a bar of text that the user
 *  can only watch is a slab of screen taken for nothing. So the unrequested
 *  download is a line at the top edge and nothing else, and the bar — the
 *  words, the version, the press — arrives with the thing it is announcing.
 *
 *  A download the user *did* ask for is the exception. They pressed something;
 *  answering with a hairline would read as the press having been dropped. */
export function updateShowsHairline(phase: UpdatePhase, requested: boolean): boolean {
  return phase === "downloading" && !requested;
}

/** The hairline's own fill, over its own fainter track. Thin and translucent:
 *  it should read as the app quietly doing something, not as chrome. */
export function updateHairlineSx(
  tint: (opacity: number) => string,
  share: number,
  animate: boolean,
): Record<string, string> {
  return updateFillSx(tint(0.55), tint(0.14), share, animate);
}
