import { type MutableRefObject, type RefObject, useEffect } from "react";
import { bindMobileSpatialDrawer } from "./mobileSpatialDrawer";
import { holdStorePresentation } from "./store";

type SettleDrawer = (
  open: boolean,
  releaseVelocity?: number,
  onSettled?: () => void,
  cachedWidth?: number,
) => void;

/** The Mobile Sessions drawer: binds the shared spatial-drawer controller to
 *  App's surfaces, and squares the drawer surface's bottom corners while the
 *  keyboard is present. App calls it where both effects used to run. */
export function useMobileDrawerSurface({
  mobile,
  phone,
  onMobileDrawerOpenChange,
  setDrawerOpen,
  columnRef,
  mobileLayerRef,
  mobilePageRef,
  mobileNavFollowRef,
  mobileComposerFollowRef,
  mobileFrostFollowRef,
  mobileStatusChromeFollowRef,
  mobileShellRef,
  mobileDrawerRef,
  mobileDrawerMaskRef,
  mobileDrawerDimRef,
  drawerOpenRef,
  settleMobileDrawerRef,
}: {
  mobile: boolean;
  phone: boolean;
  onMobileDrawerOpenChange: ((open: boolean) => void) | undefined;
  setDrawerOpen: (open: boolean) => void;
  columnRef: RefObject<HTMLDivElement | null>;
  mobileLayerRef: RefObject<HTMLDivElement | null>;
  mobilePageRef: RefObject<HTMLDivElement | null>;
  mobileNavFollowRef: RefObject<HTMLElement | null>;
  mobileComposerFollowRef: RefObject<HTMLDivElement | null>;
  mobileFrostFollowRef: RefObject<HTMLDivElement | null>;
  mobileStatusChromeFollowRef: RefObject<HTMLDivElement | null>;
  mobileShellRef: RefObject<HTMLDivElement | null>;
  mobileDrawerRef: RefObject<HTMLDivElement | null>;
  mobileDrawerMaskRef: RefObject<HTMLDivElement | null>;
  mobileDrawerDimRef: RefObject<HTMLDivElement | null>;
  drawerOpenRef: MutableRefObject<boolean>;
  settleMobileDrawerRef: MutableRefObject<SettleDrawer | null>;
}): void {
  // Mobile Sessions and Code Review use the same spatial-drawer controller.
  // Product code supplies only the side and presentation-freeze hook; sampling,
  // finger-tracking, magnetic thresholds, haptics, rubber-band, depth, settle
  // timing, and idle release stay identical on both surfaces.
  useEffect(() => {
      if (!mobile) return undefined;
      const surface = mobilePageRef.current ?? mobileLayerRef.current ??
          columnRef.current;
      const gestureTarget = mobileShellRef.current;
      const drawer = mobileDrawerRef.current;
      const drawerMask = mobileDrawerMaskRef.current;
      const dim = mobileDrawerDimRef.current;
      if (!surface || !gestureTarget || !drawer || !drawerMask || !dim) return undefined;
      const binding = bindMobileSpatialDrawer({
          gestureTarget,
          surface,
          drawer,
          drawerMask,
          dim,
          getSurface: () =>
              mobilePageRef.current ?? mobileLayerRef.current ??
              columnRef.current,
          getFollowers: () => [
              mobileDrawerDimRef.current,
              mobileFrostFollowRef.current,
              mobileComposerFollowRef.current,
              mobileNavFollowRef.current,
              mobileStatusChromeFollowRef.current,
          ],
          side: "left",
          phone,
          getOpen: () => drawerOpenRef.current,
          setOpen: (open) => {
              // The parent product pager listens in the capture phase. Publish
              // drawer ownership in this same input frame, before React's
              // state/effects run, so a quick reversal cannot both close the
              // Sessions drawer and begin the Agent -> Review transition.
              drawerOpenRef.current = open;
              onMobileDrawerOpenChange?.(open);
              setDrawerOpen(open);
          },
          holdPresentation: holdStorePresentation,
      });
      settleMobileDrawerRef.current = binding.settle;
      return () => {
          settleMobileDrawerRef.current = null;
          binding.dispose();
      };
  }, [mobile, onMobileDrawerOpenChange, phone]);

  // The native viewport owns full-screen device corners. While the keyboard
  // is present, temporarily square the drawer surface's bottom edge; on
  // release, remove only that override so an open drawer can recover its
  // shared card radius and a closed drawer remains entirely native-clipped.
  useEffect(() => {
      if (!mobile) return undefined;
      const surface = mobileLayerRef.current ?? columnRef.current;
      if (!surface) return undefined;
      let restoreTimer = 0;
      const ownsKeyboard = (): boolean => {
          const active = document.activeElement;
          return active instanceof HTMLElement &&
              active.matches("input, textarea, [contenteditable='true']");
      };
      const apply = (): void => {
          if (ownsKeyboard()) {
              surface.style.borderBottomLeftRadius = "0px";
              surface.style.borderBottomRightRadius = "0px";
          } else {
              surface.style.removeProperty("border-bottom-left-radius");
              surface.style.removeProperty("border-bottom-right-radius");
          }
      };
      const onFocusIn = (): void => {
          globalThis.clearTimeout(restoreTimer);
          apply();
      };
      const onFocusOut = (): void => {
          globalThis.clearTimeout(restoreTimer);
          restoreTimer = globalThis.setTimeout(apply, 420);
      };
      document.addEventListener("focusin", onFocusIn);
      document.addEventListener("focusout", onFocusOut);
      apply();
      return () => {
          document.removeEventListener("focusin", onFocusIn);
          document.removeEventListener("focusout", onFocusOut);
          globalThis.clearTimeout(restoreTimer);
          surface.style.removeProperty("border-bottom-left-radius");
          surface.style.removeProperty("border-bottom-right-radius");
      };
  }, [mobile]);
}
