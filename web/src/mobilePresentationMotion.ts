import { subscribeAnyDetentSheetOpen } from "./_shell/detent-sheet-open";
import { holdStorePresentation } from "./store";

/** Flatten iOS overflow tiles and backdrop filters into one cached layer.
 *  Nested `-webkit-overflow-scrolling: touch` and `backdrop-filter` each
 *  become their own compositor tile; translating a parent then relocates
 *  that whole tile tree every frame even when JS is idle. */
export const mobileCompositorFlattenSx = {
  "& .cm-scroller, & [data-transcript-session], & [data-mobile-overflow-layer]": {
    WebkitOverflowScrolling: "auto",
  },
  "& [data-detent-sheet-chrome], & [data-mobile-backdrop-chrome]": {
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


