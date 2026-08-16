import { navigationHaptic, prepareNavigationHaptic } from "./haptic";
import {
  mobileSpatialDrawerShadow,
  shouldKeepDrawerDepth,
} from "./mobileDrawerDepth";
import {
  drawerProgressAttribute,
  MOBILE_DRAWER_SETTLE_EASING,
  mobileDrawerCardVisual,
  mobileDrawerSettleDurationMs,
} from "./mobileDrawerMotion";
import {
  expandedSelection,
  hasHorizontalScroller,
  horizontalSwipe,
  inputOverlayOwnsDrawerGesture,
  isDominantVerticalPan,
  MOBILE_DRAWER_DIRECTION_LOCK_PX,
  MOBILE_DRAWER_PREPARE_PX,
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
  dim,
  getFollowers,
  getSurface,
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
  dim?: HTMLElement | null;
  // Layers that must share the peek translate but must not live inside
  // `surface`. iOS pins bottom chrome of a transformed full-screen ancestor
  // to the visual viewport; those pieces need their own translate3d.
  getFollowers?: () => Array<HTMLElement | null | undefined>;
  getSurface?: () => HTMLElement | null | undefined;
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
    prepared: boolean;
    startOffset: number;
    startOpen: boolean;
    width: number;
    lockPx: number;
    thresholdHaptic: boolean;
  } | null = null;
  let settleTimer = 0;
  let settleGen = 0;
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
    const width = gestureTarget.clientWidth || surface.clientWidth;
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
  const publishDrawerWidth = (width: number): void => {
    gestureTarget.style.setProperty(
      "--mobile-drawer-width",
      `${String(width)}px`,
    );
  };
  const origin = side === "left" ? "left center" : "right center";
  const page = (): HTMLElement => getSurface?.() ?? surface;
  const followerLayers = (): HTMLElement[] => {
    const seen = new Set<HTMLElement>([page(), drawerMask]);
    const layers: HTMLElement[] = [];
    for (const layer of getFollowers?.() ?? []) {
      if (!layer || seen.has(layer)) continue;
      seen.add(layer);
      layers.push(layer);
    }
    return layers;
  };
  const slidingLayers = (): HTMLElement[] => [page(), drawerMask, ...followerLayers()];
  const prepareLayer = (layer: HTMLElement): void => {
    layer.style.willChange = "transform";
    layer.style.transformOrigin = origin;
  };
  const applySlide = (offset: number): void => {
    const x = `${String(openingSign * offset)}px`;
    const transform = `translate3d(${x}, 0, 0)`;
    for (const layer of slidingLayers()) {
      prepareLayer(layer);
      layer.style.transform = transform;
    }
  };
  const applyOpenDepth = (): void => {
    // Never clip or shadow the full session surface. On iPhone, WebKit then
    // re-composites the Transcript/CodeMirror viewport while its transform is
    // changing, which punches holes in the peek and drops frames. The mask is
    // an opaque, content-free layer that follows the same offset, so its
    // narrow shadow preserves the depth cue without touching the heavy
    // foreground raster.
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
    // Touch callback writes only compositor properties. Flatten and store
    // holds happen in prepare(), before the first translate.
    const visual = mobileDrawerCardVisual(offset, presentationWidth, phone);
    applySlide(offset);
    if (dim) dim.style.opacity = String(visual.dim);
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
    settleGen += 1;
    globalThis.clearTimeout(settleTimer);
    for (const layer of slidingLayers()) layer.style.transition = "none";
    if (dim) dim.style.transition = "none";
    gestureTarget.setAttribute("data-mobile-drawer-moving", "true");
    applyOpenDepth();
    releasePresentation ??= holdPresentation?.();
    directManipulationActive = true;
    globalThis.dispatchEvent(
      new CustomEvent("cowboy:transcript-direct-manipulation-start"),
    );
  };
  const clearTransitions = (): void => {
    for (const layer of slidingLayers()) layer.style.removeProperty("transition");
    if (dim) dim.style.removeProperty("transition");
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
    publishDrawerWidth(width);
    const targetOffset = open ? width : 0;
    publishProgress(targetOffset);
    if (pendingThresholdHaptic) {
      pendingThresholdHaptic = false;
      requestAnimationFrame(() => navigationHaptic());
    }
    if (open) setOpen(true);
    globalThis.clearTimeout(settleTimer);
    const generation = settleGen += 1;
    const remaining = Math.min(
      1,
      Math.abs(targetOffset - releaseOffset) / Math.max(1, width),
    );
    const duration = mobileDrawerSettleDurationMs(remaining, releaseVelocity);
    const transition =
      `transform ${String(duration)}ms ${MOBILE_DRAWER_SETTLE_EASING}`;
    for (const layer of slidingLayers()) {
      if (layer === dim) continue;
      layer.style.transition = transition;
    }
    if (dim) {
      dim.style.transition =
        `transform ${String(duration)}ms ${MOBILE_DRAWER_SETTLE_EASING}, opacity ${String(duration)}ms ${MOBILE_DRAWER_SETTLE_EASING}`;
    }
    const finish = (): void => {
      if (generation !== settleGen) return;
      globalThis.clearTimeout(settleTimer);
      render(targetOffset);
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
    };
    render(targetOffset);
    settleTimer = globalThis.setTimeout(finish, duration + 20);
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
    const width = presentationWidth > 1 ? presentationWidth : drawerWidth();
    presentationWidth = width;
    publishDrawerWidth(width);
    prepareNavigationHaptic();
    // Session-row taps live on the rail and need the 12px lock. The peeking
    // page must track as soon as horizontal intent is visible, like Obsidian.
    const lockPx = target && drawer.contains(target)
      ? MOBILE_DRAWER_DIRECTION_LOCK_PX
      : MOBILE_DRAWER_PREPARE_PX;
    gesture = {
      x: touch.clientX,
      y: touch.clientY,
      lastX: touch.clientX,
      lastAt: now,
      velocity: 0,
      locked: false,
      prepared: false,
      startOffset: startOpen ? width : 0,
      startOpen,
      width,
      lockPx,
      thresholdHaptic: false,
    };
    commit = startOpen;
  };
  const onTouchMove = (event: TouchEvent): void => {
    const touch = event.touches[0];
    if (!gesture || !touch) return;
    if (
      !gesture.locked &&
      expandedSelection(globalThis.getSelection?.() ?? null)
    ) {
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
    const opening = !gesture.startOpen;
    const towardOpen = opening
      ? normalizedDelta > 0
      : normalizedDelta < 0;
    if (
      !gesture.prepared &&
      towardOpen &&
      Math.abs(normalizedDelta) >= MOBILE_DRAWER_PREPARE_PX &&
      Math.abs(normalizedDelta) > Math.abs(deltaY)
    ) {
      // Assemble one compositor layer before the first translate. Obsidian's
      // workspace is already that layer; doing this at lock time hitchs.
      gesture.prepared = true;
      beginDirectManipulation();
    }
    const swipe = gesture.locked
      ? {
        direction: normalizedDelta < 0 ? "left" as const : "right" as const,
        distance: Math.abs(normalizedDelta),
      }
      : horizontalSwipe(
        normalizedDelta,
        deltaY,
        gesture.lockPx,
        1.15,
      );
    if (
      !swipe ||
      (opening && swipe.direction !== "right") ||
      (!opening && swipe.direction !== "left")
    ) return;
    if (!gesture.locked) {
      gesture.locked = true;
      publishProgress(gesture.startOffset + normalizedDelta);
    }
    event.preventDefault();
    event.stopPropagation();
    const now = performance.now();
    const elapsed = Math.max(1, now - gesture.lastAt);
    const instantaneousVelocity = (touch.clientX - gesture.lastX) / elapsed *
      openingSign;
    gesture.velocity = gesture.velocity * 0.35 +
      instantaneousVelocity * 0.65;
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
  };
  const onTouchEnd = (): void => {
    if (!gesture) return;
    if (!gesture.locked) {
      if (gesture.prepared) releaseDirectManipulation();
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
    const wasPrepared = gesture?.prepared === true;
    const startOpen = gesture?.startOpen ?? getOpen();
    const width = gesture?.width;
    gesture = null;
    commit = false;
    if (wasLocked) settle(startOpen, 0, releaseDirectManipulation, width);
    else if (wasPrepared) releaseDirectManipulation();
  };
  const onResize = (): void => {
    presentationWidth = drawerWidth();
    publishDrawerWidth(presentationWidth);
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
  publishDrawerWidth(presentationWidth);
  // Obsidian's rail is its own layer and never translates. If this drawer
  // flattens into the sliding page, the session list rides with the peek.
  drawer.style.transform = "translate3d(0, 0, 0)";
  drawer.style.isolation = "isolate";
  surface.style.removeProperty("border-radius");
  surface.style.removeProperty("overflow");
  for (const layer of slidingLayers()) prepareLayer(layer);
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
      gestureTarget.removeAttribute("data-mobile-drawer-moving");
      gestureTarget.style.removeProperty("--mobile-drawer-width");
      drawer.style.removeProperty("transform");
      drawer.style.removeProperty("isolation");
      for (const layer of slidingLayers()) {
        layer.style.removeProperty("will-change");
        if (layer !== surface && layer !== drawerMask) {
          layer.style.removeProperty("transform");
          layer.style.removeProperty("transform-origin");
        }
      }
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
