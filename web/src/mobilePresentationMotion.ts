import { subscribeAnyDetentSheetOpen } from "./_shell/detent-sheet-open";
import { holdStorePresentation } from "./store";

/** Standing peek paint collapse. Settled rows each own `contain: layout
 *  paint`; leaving that in place until the 2 px claim restyles N tiles on
 *  the same frames as the first translate. Overflow and frost stay live so
 *  vertical scroll and Working marks still work at rest. */
export const mobilePeekRestLayerSx = {
  "& [data-mobile-drawer-surface] [data-key]": {
    contain: "none",
  },
};

/** Strip backdrop-filter only when the moving surface *contains* the frost
 *  (product pager, detent sheet). A dedicated drawer frost follower already
 *  has its own translate3d; toggling its filter at prepare rebuilds that
 *  layer and is the intermittent first-frame hitch. */
export const mobileFrostStripSx = {
  "& [data-detent-sheet-chrome], & [data-mobile-backdrop-chrome], & [data-mobile-composer-shell-material], & [data-mobile-focus-composer], & [data-mobile-primary-composer], & [data-mobile-pending-editor]": {
    backdropFilter: "none",
    WebkitBackdropFilter: "none",
  },
};

/** Flatten iOS overflow tiles into one cached layer.
 *  Nested `-webkit-overflow-scrolling: touch` each become their own
 *  compositor tile; translating a parent then relocates that whole tile
 *  tree every frame even when JS is idle. Apply this only after the
 *  swipe is claimed — overflow:hidden would break vertical scroll. */
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
  // Settled rows each own a paint layer. Flatten them into the peek so a
  // long transcript is one tile during the swipe, not N independent ones.
  "& [data-mobile-drawer-surface] [data-key]": {
    contain: "none",
  },
  "& [data-mobile-drawer-surface] .MuiCircularProgress-root, & [data-mobile-drawer-surface] .MuiSkeleton-root, & [data-mobile-drawer-surface] [data-mobile-css-animation]": {
    animationPlayState: "paused",
  },
};

export function mobilePresentationMovingRootSx(
  attr: "data-mobile-drawer-moving" | "data-mobile-product-moving",
): Record<string, typeof mobileCompositorFlattenSx | (
  & typeof mobileCompositorFlattenSx
  & typeof mobileFrostStripSx
)> {
  return {
    [`&[${attr}='true']`]: attr === "data-mobile-product-moving"
      ? { ...mobileCompositorFlattenSx, ...mobileFrostStripSx }
      : mobileCompositorFlattenSx,
  };
}

/** Settled-open hit testing. Peek layers stay full-width in layout and
 *  only translate visually, so iOS would send rail taps to the page.
 *  The painted dim keeps its swipe translate — resizing it after settle
 *  restretches the gradient and makes the veil jump. A separate close
 *  hit layer covers only the peek. */
export const mobileDrawerRailHitSx = {
  "&[data-mobile-drawer-open='true']:not([data-mobile-drawer-moving='true']) [data-mobile-drawer-surface], &[data-mobile-drawer-open='true']:not([data-mobile-drawer-moving='true']) [data-mobile-drawer-follow]": {
    pointerEvents: "none",
  },
  "& [data-mobile-drawer-dim]": {
    pointerEvents: "none",
  },
  "&[data-mobile-drawer-open='true']:not([data-mobile-drawer-moving='true']) [data-mobile-drawer-close]": {
    pointerEvents: "auto",
  },
  "&[data-mobile-drawer-open='true']:not([data-mobile-drawer-moving='true']) [data-mobile-drawer-close='left']": {
    left: "var(--mobile-drawer-width, min(84%, 360px))",
  },
  "&[data-mobile-drawer-open='true']:not([data-mobile-drawer-moving='true']) [data-mobile-drawer-close='right']": {
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
  "&[data-mobile-sheet-presented='true']": {
    ...mobileCompositorFlattenSx,
    ...mobileFrostStripSx,
  },
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


