import { navigationHaptic, prepareNavigationHaptic } from "./haptic";
import {
  mobileSpatialDrawerShadow,
  shouldKeepDrawerDepth,
} from "./mobileDrawerDepth";
import {
  drawerProgressAttribute,
  MOBILE_DRAWER_SETTLE_EASING,
  mobileDrawerSettleDurationMs,
} from "./mobileDrawerMotion";
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
  let bookkeepingFrame = 0;
  let pendingThresholdHaptic = false;
  let currentOffset = 0;
  let commit = false;
  let directManipulationActive = false;
  let releaseFrame = 0;
  let releaseIdle: number | undefined;
  let releasePresentation: (() => void) | undefined;
  let presentationWidth = 1;
  let lastPublishedProgress: string | null = null;
  const drawerWidth = (): number => {
    const width = surface.clientWidth;
    return phone ? Math.min(360, width * 0.84) : Math.min(440, width * 0.52);
  };
  const publishProgress = (offset: number): void => {
    const width = Math.max(1, presentationWidth);
    const next = drawerProgressAttribute(Math.abs(offset) / width);
    if (next === lastPublishedProgress) return;
    lastPublishedProgress = next;
    if (next) {
      gestureTarget.setAttribute("data-mobile-drawer-progress", next);
    } else {
      gestureTarget.removeAttribute("data-mobile-drawer-progress");
    }
  };
  const applyOpenDepth = (): void => {
    // Never clip or shadow the full session surface. On iPhone, WebKit then
    // re-composites the Transcript/CodeMirror viewport while its transform is
    // changing, which turns a compositor-only drag into visible frame drops.
    // The mask is an opaque, content-free layer that follows the same offset,
    // so its narrow shadow preserves the depth cue without touching the heavy
    // foreground raster. iPad usually has enough GPU headroom to hide the
    // mistake, but keeping both sizes on the cheap path is more predictable.
    drawerMask.style.boxShadow = mobileSpatialDrawerShadow(side);
  };
  const clearOpenDepth = (): void => {
    if (shouldKeepDrawerDepth(getOpen(), currentOffset)) {
      applyOpenDepth();
      return;
    }
    drawerMask.style.removeProperty("box-shadow");
  };
  const render = (offset: number): void => {
    currentOffset = offset;
    publishProgress(offset);
    // Obsidian's workspace-drawer is 1:1 with the finger: the heavy surface
    // translates, the rail stays put, and nothing else is dirtied in the
    // touch callback. Parallax/opacity/scale force extra compositor work and
    // make the card float ahead of the finger.
    const x = `${String(openingSign * offset)}px`;
    surface.style.transform = `translate3d(${x}, 0, 0)`;
    drawerMask.style.transform = `translate3d(${x}, 0, 0)`;
  };
  const beginDirectManipulation = (): void => {
    if (directManipulationActive) return;
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
    const releaseOffset = currentOffset;
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
    const duration = mobileDrawerSettleDurationMs(remaining, releaseVelocity);
    const settleTransition =
      `transform ${String(duration)}ms ${MOBILE_DRAWER_SETTLE_EASING}`;
    surface.style.transition = settleTransition;
    drawerMask.style.transition = settleTransition;
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
      surface.style.transition = "none";
      surface.style.willChange = "transform";
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
    const progress = Math.max(0, Math.min(1, offset / width));
    const nextCommit = gesture.startOpen ? progress > 0.66 : progress >= 0.34;
    if (nextCommit !== commit && !gesture.thresholdHaptic) {
      gesture.thresholdHaptic = true;
      pendingThresholdHaptic = true;
    }
    commit = nextCommit;
    render(offset);
    if (pendingThresholdHaptic) {
      pendingThresholdHaptic = false;
      navigationHaptic();
    }
    if (bookkeepingFrame === 0 && !directManipulationActive) {
      bookkeepingFrame = requestAnimationFrame(() => {
        bookkeepingFrame = 0;
        beginDirectManipulation();
      });
    }
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
      if (bookkeepingFrame !== 0) cancelAnimationFrame(bookkeepingFrame);
      gestureTarget.removeAttribute("data-mobile-drawer-moving");
      lastPublishedProgress = null;
      if (releaseFrame !== 0) cancelAnimationFrame(releaseFrame);
      if (releaseIdle !== undefined) {
        if (typeof globalThis.cancelIdleCallback === "function") {
          globalThis.cancelIdleCallback(releaseIdle);
        } else {
          globalThis.clearTimeout(releaseIdle);
        }
      }
      if (!shouldKeepDrawerDepth(getOpen(), currentOffset)) {
        gestureTarget.removeAttribute("data-mobile-drawer-open");
        gestureTarget.removeAttribute("data-mobile-drawer-progress");
        drawerMask.style.removeProperty("box-shadow");
      }
    },
  };
}
