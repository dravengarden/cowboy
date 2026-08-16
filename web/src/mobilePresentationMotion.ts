import { subscribeAnyDetentSheetOpen } from "./_shell/detent-sheet-open";
import { holdStorePresentation } from "./store";

/** Flatten iOS overflow tiles and backdrop filters into one cached layer.
 *  Nested `-webkit-overflow-scrolling: touch` and `backdrop-filter` each
 *  become their own compositor tile; translating a parent then relocates
 *  that whole tile tree every frame even when JS is idle. */
export const mobileCompositorFlattenSx = {
  // Only the peeking page. The Sessions/Review rail uses the same
  // overflow-layer marker so it can scroll; freezing it while the
  // drawer is open makes every session row un-tappable.
  "& [data-mobile-drawer-surface] .cm-scroller, & [data-mobile-drawer-surface] [data-transcript-session], & [data-mobile-drawer-surface] [data-mobile-overflow-layer]": {
    WebkitOverflowScrolling: "auto",
    overflow: "hidden",
    contain: "paint",
    pointerEvents: "none",
  },
  "& [data-detent-sheet-chrome], & [data-mobile-backdrop-chrome], & [data-mobile-focus-composer], & [data-mobile-primary-composer], & [data-mobile-pending-editor]": {
    backdropFilter: "none",
    WebkitBackdropFilter: "none",
  },
  "& .MuiCircularProgress-root, & .MuiSkeleton-root, & [data-mobile-css-animation]": {
    animationPlayState: "paused",
  },
};

export function mobilePresentationMovingRootSx(
  attr: "data-mobile-drawer-moving" | "data-mobile-product-moving",
): Record<string, typeof mobileCompositorFlattenSx> {
  return {
    [`&[${attr}='true']`]: mobileCompositorFlattenSx,
  };
}

/** Settled-open hit testing. Peek layers stay full-width in layout and
 *  only translate visually, so iOS would send rail taps to the dim/page.
 *  Park the dim on the peek with a layout inset and let the rail receive
 *  the rest. */
export const mobileDrawerRailHitSx = {
  "&[data-mobile-drawer-open='true']:not([data-mobile-drawer-moving='true']) [data-mobile-drawer-surface], &[data-mobile-drawer-open='true']:not([data-mobile-drawer-moving='true']) [data-mobile-drawer-follow]": {
    pointerEvents: "none",
  },
  "&[data-mobile-drawer-open='true']:not([data-mobile-drawer-moving='true']) [data-mobile-drawer-dim]": {
    pointerEvents: "auto",
    transform: "none !important",
  },
  "&[data-mobile-drawer-open='true']:not([data-mobile-drawer-moving='true']) [data-mobile-drawer-dim='left']": {
    left: "var(--mobile-drawer-width, min(84%, 360px))",
  },
  "&[data-mobile-drawer-open='true']:not([data-mobile-drawer-moving='true']) [data-mobile-drawer-dim='right']": {
    right: "var(--mobile-drawer-width, min(84%, 360px))",
  },
};

/** DetentSheet already disables its own blur while `data-detent-moving` is
 *  set. The remaining hitch is the Paper elevation shadow plus the page
 *  behind the sheet (CM6 + frosted chrome) still being independent tiles. */
export const mobileSheetPresentationSx = {
  "& [data-detent-sheet][data-detent-moving]": {
    boxShadow: "none !important",
    backdropFilter: "none !important",
    WebkitBackdropFilter: "none !important",
  },
  "&[data-mobile-sheet-presented='true']": mobileCompositorFlattenSx,
};

/** Freeze store subscribers and flatten the page for the whole time a sheet
 *  is registered. DetentSheet only calls `onClose` after the dismiss settle,
 *  so this window covers both the open and close slides. */
export function bindMobileSheetPresentationHold(
  root: HTMLElement,
): () => void {
  let release: (() => void) | undefined;
  let announced = false;
  const apply = (open: boolean): void => {
    if (open) {
      release ??= holdStorePresentation();
      root.setAttribute("data-mobile-sheet-presented", "true");
      if (!announced) {
        announced = true;
        globalThis.dispatchEvent(
          new CustomEvent("cowboy:transcript-direct-manipulation-start"),
        );
      }
      return;
    }
    if (announced) {
      announced = false;
      globalThis.dispatchEvent(
        new CustomEvent("cowboy:transcript-direct-manipulation-end"),
      );
    }
    release?.();
    release = undefined;
    root.removeAttribute("data-mobile-sheet-presented");
  };
  const unsubscribe = subscribeAnyDetentSheetOpen(apply);
  return () => {
    unsubscribe();
    apply(false);
  };
}


