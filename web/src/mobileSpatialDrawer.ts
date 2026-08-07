import { navigationHaptic, prepareNavigationHaptic } from "./haptic";
import { mobileDrawerProgress, predictDrawerOffset } from "./mobileDrawerMotion";
import {
  expandedSelection,
  hasHorizontalScroller,
  horizontalSwipe,
  inputOverlayOwnsDrawerGesture,
  isDominantVerticalPan,
  MOBILE_DRAWER_DIRECTION_LOCK_PX,
} from "./touchGestures";

export type MobileSpatialDrawerSide = "left" | "right";
export type MobileSpatialDrawerSettle = (
  open: boolean,
  releaseVelocity?: number,
  onSettled?: () => void,
  cachedWidth?: number,
) => void;

export interface MobileSpatialDrawerBinding {
  settle: MobileSpatialDrawerSettle;
  dispose: () => void;
}

export function bindMobileSpatialDrawer({
  gestureTarget,
  surface,
  drawer,
  drawerMask,
  side,
  phone,
  getOpen,
  setOpen,
  holdPresentation,
  ignoreSelector = "[data-mobile-drawer-ignore]",
}: {
  gestureTarget: HTMLElement;
  surface: HTMLElement;
  drawer: HTMLElement;
  drawerMask: HTMLElement;
  side: MobileSpatialDrawerSide;
  phone: boolean;
  getOpen: () => boolean;
  setOpen: (open: boolean) => void;
  holdPresentation?: () => () => void;
  ignoreSelector?: string;
}): MobileSpatialDrawerBinding {
  const openingSign = side === "left" ? 1 : -1;
  let gesture: {
    x: number;
    y: number;
    lastX: number;
    lastAt: number;
    velocity: number;
    locked: boolean;
    startOffset: number;
    startOpen: boolean;
    width: number;
    thresholdHaptic: boolean;
  } | null = null;
  let settleTimer = 0;
  let renderFrame = 0;
  let pendingOffset = 0;
  let pendingSampleAt = 0;
  let pendingVelocity = 0;
  let pendingThresholdHaptic = false;
  let currentOffset = 0;
  let commit = false;
  let directManipulationActive = false;
  let releaseFrame = 0;
  let releaseIdle: number | undefined;
  let releasePresentation: (() => void) | undefined;
  let presentationWidth = 1;
  const drawerWidth = (): number => {
    const width = surface.clientWidth;
    return phone ? Math.min(360, width * 0.84) : Math.min(440, width * 0.52);
  };
  const applyOpenDepth = (): void => {
    surface.style.borderRadius = `${String(phone ? 36 : 30)}px`;
    surface.style.boxShadow = `${
      side === "left" ? "-" : ""
    }18px 0 42px rgba(0,0,0,0.16)`;
  };
  const clearOpenDepth = (): void => {
    surface.style.removeProperty("border-radius");
    surface.style.removeProperty("box-shadow");
  };
  const render = (offset: number): void => {
    currentOffset = offset;
    const progress = mobileDrawerProgress(offset, presentationWidth);
    const drawerParallax = presentationWidth * (phone ? 0.28 : 0.22) *
      (1 - progress);
    // Keep the heavy foreground (Transcript or CodeMirror) on a translation-
    // only compositor path. Scaling or fading this layer makes iPhone WebKit
    // blend/raster the full viewport for every touch frame. The static radius
    // and edge shadow plus the lightweight drawer parallax still provide the
    // same spatial depth without spending the phone's frame budget.
    surface.style.transform = `translate3d(${
      String(openingSign * offset)
    }px, 0, 0)`;
    drawer.style.transform = `translate3d(${
      String(-openingSign * drawerParallax)
    }px, 0, 0)`;
    drawer.style.opacity = String(0.72 + progress * 0.28);
    drawerMask.style.transform = `translate3d(${
      String(openingSign * offset)
    }px, 0, 0)`;
  };
  const scheduleRender = (
    offset: number,
    sampleAt: number,
    velocity: number,
  ): void => {
    pendingOffset = offset;
    pendingSampleAt = sampleAt;
    pendingVelocity = velocity;
    if (renderFrame !== 0) return;
    renderFrame = requestAnimationFrame((frameAt) => {
      renderFrame = 0;
      render(predictDrawerOffset(
        pendingOffset,
        pendingVelocity,
        frameAt - pendingSampleAt,
      ));
      if (pendingThresholdHaptic) {
        pendingThresholdHaptic = false;
        navigationHaptic();
      }
    });
  };
  const clearTransitions = (): void => {
    for (const element of [surface, drawer, drawerMask]) {
      element.style.removeProperty("transition");
      element.style.removeProperty("will-change");
    }
  };
  const releaseDirectManipulation = (): void => {
    const finish = (): void => {
      releaseIdle = undefined;
      directManipulationActive = false;
      gestureTarget.removeAttribute("data-mobile-drawer-moving");
      releasePresentation?.();
      releasePresentation = undefined;
      globalThis.dispatchEvent(
        new CustomEvent("cowboy:transcript-direct-manipulation-end"),
      );
    };
    releaseFrame = requestAnimationFrame(() => {
      releaseFrame = 0;
      if (typeof globalThis.requestIdleCallback === "function") {
        releaseIdle = globalThis.requestIdleCallback(finish, { timeout: 180 });
      } else {
        releaseIdle = globalThis.setTimeout(finish, 32);
      }
    });
  };
  const settle: MobileSpatialDrawerSettle = (
    open,
    releaseVelocity = 0,
    onSettled,
    cachedWidth,
  ): void => {
    globalThis.clearTimeout(settleTimer);
    // Publish gesture ownership synchronously, before React state/effects can
    // run. Closing a left drawer uses the same direction as Agent -> Review;
    // closing a right drawer uses the same direction as Review -> Agent. Keep
    // the drawer marked open through its complete close animation so the
    // outer product pager can never steal either gesture.
    if (open) gestureTarget.setAttribute("data-mobile-drawer-open", "true");
    const releaseOffset = renderFrame !== 0 ? pendingOffset : currentOffset;
    if (renderFrame !== 0) cancelAnimationFrame(renderFrame);
    renderFrame = 0;
    const width = cachedWidth ?? drawerWidth();
    // Read geometry before invalidating styles. CodeMirror makes a forced
    // layout here particularly expensive on Review, and Agent benefits from
    // keeping the same read-then-write phase ordering.
    applyOpenDepth();
    presentationWidth = width;
    const targetOffset = open ? width : 0;
    const remaining = Math.min(
      1,
      Math.abs(targetOffset - releaseOffset) / width,
    );
    const duration = Math.max(
      150,
      Math.min(
        260,
        160 + remaining * 100 -
          Math.min(70, Math.abs(releaseVelocity) * 45),
      ),
    );
    surface.style.transition =
      `transform ${String(duration)}ms cubic-bezier(0.22, 1, 0.36, 1)`;
    drawerMask.style.transition =
      `transform ${String(duration)}ms cubic-bezier(0.22, 1, 0.36, 1)`;
    drawer.style.transition =
      `transform ${String(duration)}ms cubic-bezier(0.22, 1, 0.36, 1), ` +
      `opacity ${String(duration)}ms cubic-bezier(0.22, 1, 0.36, 1)`;
    render(targetOffset);
    if (pendingThresholdHaptic) {
      pendingThresholdHaptic = false;
      requestAnimationFrame(() => navigationHaptic());
    }
    if (open) setOpen(true);
    settleTimer = globalThis.setTimeout(() => {
      clearTransitions();
      if (!open) {
        setOpen(false);
        gestureTarget.removeAttribute("data-mobile-drawer-open");
        // A closed surface is the native full-screen viewport. Let UIKit and
        // WKWebView clip it to the real device corners instead of leaving a
        // guessed phone/tablet radius that cuts a visible wedge from iPad.
        clearOpenDepth();
      }
      onSettled?.();
    }, duration + 20);
  };

  const onTouchStart = (event: TouchEvent): void => {
    const touch = event.touches[0];
    const target = event.target instanceof HTMLElement ? event.target : null;
    const inputOverlay = target?.closest("[data-mobile-keyboard-open]");
    const focusedInputOverlayOwnsGesture = inputOverlayOwnsDrawerGesture(
      inputOverlay?.getAttribute("data-mobile-keyboard-open") === "true",
      inputOverlay?.matches(":focus-within") === true,
    );
    if (
      !touch ||
      expandedSelection(globalThis.getSelection?.() ?? null) ||
      focusedInputOverlayOwnsGesture ||
      target?.closest(ignoreSelector) ||
      hasHorizontalScroller(event.target, gestureTarget)
    ) {
      gesture = null;
      return;
    }
    const now = performance.now();
    const startOpen = getOpen();
    const width = drawerWidth();
    presentationWidth = width;
    prepareNavigationHaptic();
    gesture = {
      x: touch.clientX,
      y: touch.clientY,
      lastX: touch.clientX,
      lastAt: now,
      velocity: 0,
      locked: false,
      startOffset: startOpen ? width : 0,
      startOpen,
      width,
      thresholdHaptic: false,
    };
    commit = startOpen;
  };
  const onTouchMove = (event: TouchEvent): void => {
    const touch = event.touches[0];
    if (!gesture || !touch) return;
    if (expandedSelection(globalThis.getSelection?.() ?? null)) {
      gesture = null;
      commit = false;
      return;
    }
    const deltaX = touch.clientX - gesture.x;
    const deltaY = touch.clientY - gesture.y;
    if (!gesture.locked && isDominantVerticalPan(deltaX, deltaY)) {
      gesture = null;
      // Preserve the transcript's native vertical scroll while preventing a
      // parent horizontal recognizer from seeing the same touch stream.
      event.stopPropagation();
      return;
    }
    const normalizedDelta = deltaX * openingSign;
    const swipe = gesture.locked
      ? {
        direction: normalizedDelta < 0 ? "left" as const : "right" as const,
        distance: Math.abs(normalizedDelta),
      }
      : horizontalSwipe(
        normalizedDelta,
        deltaY,
        MOBILE_DRAWER_DIRECTION_LOCK_PX,
        1.15,
      );
    if (
      !swipe ||
      (!gesture.startOpen && swipe.direction !== "right") ||
      (gesture.startOpen && swipe.direction !== "left")
    ) return;
    if (!gesture.locked) {
      if (releaseFrame !== 0) cancelAnimationFrame(releaseFrame);
      if (releaseIdle !== undefined) {
        if (typeof globalThis.cancelIdleCallback === "function") {
          globalThis.cancelIdleCallback(releaseIdle);
        } else {
          globalThis.clearTimeout(releaseIdle);
        }
        releaseIdle = undefined;
      }
      globalThis.dispatchEvent(
        new CustomEvent("cowboy:transcript-direct-manipulation-start"),
      );
      releasePresentation ??= holdPresentation?.();
      directManipulationActive = true;
      gestureTarget.setAttribute("data-mobile-drawer-moving", "true");
      applyOpenDepth();
      surface.style.transition = "none";
      surface.style.willChange = "transform";
      drawer.style.transition = "none";
      drawer.style.willChange = "transform, opacity";
      drawerMask.style.transition = "none";
      drawerMask.style.willChange = "transform";
    }
    gesture.locked = true;
    event.preventDefault();
    event.stopPropagation();
    const now = performance.now();
    const elapsed = Math.max(1, now - gesture.lastAt);
    const instantaneousVelocity = (touch.clientX - gesture.lastX) / elapsed *
      openingSign;
    gesture.velocity = gesture.velocity * 0.65 +
      instantaneousVelocity * 0.35;
    gesture.lastX = touch.clientX;
    gesture.lastAt = now;
    const width = gesture.width;
    let offset = gesture.startOffset + normalizedDelta;
    if (offset < 0) offset *= 0.18;
    if (offset > width) offset = width + (offset - width) * 0.18;
    scheduleRender(offset, now, gesture.velocity);
    const progress = Math.max(0, Math.min(1, offset / width));
    const nextCommit = gesture.startOpen ? progress > 0.66 : progress >= 0.34;
    if (nextCommit !== commit && !gesture.thresholdHaptic) {
      pendingThresholdHaptic = true;
      gesture.thresholdHaptic = true;
    }
    commit = nextCommit;
  };
  const onTouchEnd = (): void => {
    if (!gesture) return;
    if (!gesture.locked) {
      gesture = null;
      commit = false;
      return;
    }
    const startOpen = gesture.startOpen;
    const width = gesture.width;
    const velocityCommit = startOpen
      ? gesture.velocity > -0.45
      : gesture.velocity > 0.45;
    const releaseVelocity = gesture.velocity;
    const shouldOpen = Math.abs(gesture.velocity) >= 0.45
      ? velocityCommit
      : commit;
    if (shouldOpen !== commit && !gesture.thresholdHaptic) {
      navigationHaptic();
    }
    gesture = null;
    commit = false;
    settle(shouldOpen, releaseVelocity, releaseDirectManipulation, width);
  };
  const onTouchCancel = (): void => {
    const wasLocked = gesture?.locked === true;
    const startOpen = gesture?.startOpen ?? getOpen();
    const width = gesture?.width;
    gesture = null;
    commit = false;
    if (wasLocked) settle(startOpen, 0, releaseDirectManipulation, width);
  };
  const onResize = (): void => {
    presentationWidth = drawerWidth();
    render(getOpen() ? presentationWidth : 0);
  };

  if (getOpen()) {
    gestureTarget.setAttribute("data-mobile-drawer-open", "true");
    applyOpenDepth();
  } else {
    gestureTarget.removeAttribute("data-mobile-drawer-open");
    clearOpenDepth();
  }
  presentationWidth = drawerWidth();
  render(getOpen() ? presentationWidth : 0);
  gestureTarget.addEventListener("touchstart", onTouchStart, { passive: true });
  gestureTarget.addEventListener("touchmove", onTouchMove, { passive: false });
  gestureTarget.addEventListener("touchend", onTouchEnd, { passive: true });
  gestureTarget.addEventListener("touchcancel", onTouchCancel, { passive: true });
  globalThis.addEventListener("resize", onResize);

  return {
    settle,
    dispose: () => {
      if (gesture?.locked || directManipulationActive) {
        globalThis.dispatchEvent(
          new CustomEvent("cowboy:transcript-direct-manipulation-end"),
        );
      }
      releasePresentation?.();
      gestureTarget.removeEventListener("touchstart", onTouchStart);
      gestureTarget.removeEventListener("touchmove", onTouchMove);
      gestureTarget.removeEventListener("touchend", onTouchEnd);
      gestureTarget.removeEventListener("touchcancel", onTouchCancel);
      globalThis.removeEventListener("resize", onResize);
      globalThis.clearTimeout(settleTimer);
      if (renderFrame !== 0) cancelAnimationFrame(renderFrame);
      gestureTarget.removeAttribute("data-mobile-drawer-moving");
      gestureTarget.removeAttribute("data-mobile-drawer-open");
      if (releaseFrame !== 0) cancelAnimationFrame(releaseFrame);
      if (releaseIdle !== undefined) {
        if (typeof globalThis.cancelIdleCallback === "function") {
          globalThis.cancelIdleCallback(releaseIdle);
        } else {
          globalThis.clearTimeout(releaseIdle);
        }
      }
      clearOpenDepth();
    },
  };
}
