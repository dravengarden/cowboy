// DetentSheet open-registry — a module-level count of currently-open sheets, so
// OTHER fixed-position chrome (e.g. a floating action puck) can recede while a
// sheet is up.
//
// Load-bearing because DetentSheet renders INLINE (no portal — see its header):
// its z-index is trapped in whatever ancestor stacking context it sits in, so a
// root-level `position: fixed` element can paint OVER it. Rather than portal
// (which breaks the iframe-host case the inline design exists for), let those
// elements subscribe and hide themselves while a sheet is open. This is purely a
// JS counter + listeners — ZERO document mutation — so it preserves the
// no-side-effects property the inline design depends on.

import { useEffect, useState } from "react";

let openCount = 0;
// Monotonic stack level handed to each sheet as it opens, so a later sheet's
// z-index (scrim + surface) sits strictly ABOVE every currently-open sheet — the
// fix for "the upper sheet's scrim doesn't cover the lower sheet". A monotonic
// counter (not openCount) so an out-of-order close can't hand a new sheet a level
// that collides with one still open. Reset to 0 once the stack fully drains, so
// z stays bounded across many open/close cycles (never creeps into the modal
// band). Real stacks are 2-3 deep.
let nextLevel = 0;
const activeLevels = new Set<number>();
const listeners = new Set<(open: boolean) => void>();
const stackListeners = new Set<() => void>();

function notify(): void {
  const anyOpen = openCount > 0;
  listeners.forEach((l) => l(anyOpen));
  stackListeners.forEach((l) => l());
}

/** Called by DetentSheet while it is open. Returns this sheet's `level` in the
 *  open stack (0 = bottom-most) — which drives its z-index so a later sheet
 *  occludes an earlier one — and the matching `close` decrement for cleanup. */
export function markDetentSheetOpen(): { level: number; close: () => void } {
  const level = nextLevel;
  nextLevel += 1;
  openCount += 1;
  activeLevels.add(level);
  notify();
  let closed = false;
  return {
    level,
    close: (): void => {
      if (closed) {
        return;
      }
      closed = true;
      activeLevels.delete(level);
      openCount = activeLevels.size;
      if (openCount <= 0) {
        openCount = 0;
        activeLevels.clear();
        nextLevel = 0; // stack fully drained → reset levels (keep z bounded)
      }
      notify();
    },
  };
}

/** Subscribe to "is any DetentSheet open?". Fires immediately with the current
 *  value, then on every change. Returns an unsubscribe. */
export function subscribeAnyDetentSheetOpen(
  onChange: (open: boolean) => void,
): () => void {
  listeners.add(onChange);
  onChange(openCount > 0);
  return () => {
    listeners.delete(onChange);
  };
}

/** Synchronous query for gesture arbitration. Unlike the hook/subscription,
 *  this can be sampled in the same touchstart that decides whether a
 *  background surface may claim a swipe. */
export function isAnyDetentSheetOpen(): boolean {
  return openCount > 0;
}

/** True when this sheet owns the top-most active stack level. Parent sheets
 * must hide their fixed footer while a nested sheet is open: DetentSheet is
 * intentionally inline and an ancestor stacking context can otherwise paint
 * the parent's footer above the nested surface. */
export function isTopDetentSheet(level: number): boolean {
  if (activeLevels.size === 0) {
    return false;
  }
  let top = -1;
  for (const active of activeLevels) {
    top = Math.max(top, active);
  }
  return top === level;
}

/** Subscribe to stack-depth changes. Unlike the boolean registry this keeps
 * each DetentSheet's footer synchronized when a child opens or closes. */
export function subscribeDetentSheetStack(onChange: () => void): () => void {
  stackListeners.add(onChange);
  onChange();
  return () => {
    stackListeners.delete(onChange);
  };
}

/** React hook for whether one particular DetentSheet is top-most. */
export function useIsTopDetentSheet(level: number, pinTop = false): boolean {
  const [top, setTop] = useState(() => pinTop || isTopDetentSheet(level));
  useEffect(
    () => subscribeDetentSheetStack(() => setTop(pinTop || isTopDetentSheet(level))),
    [level, pinTop],
  );
  return top;
}

/** React hook: true while at least one DetentSheet is open anywhere. */
export function useAnyDetentSheetOpen(): boolean {
  const [open, setOpen] = useState(openCount > 0);
  useEffect(() => subscribeAnyDetentSheetOpen(setOpen), []);
  return open;
}
