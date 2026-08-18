import { Box } from "@mui/material";
import { useCallback, useEffect, useRef, useState } from "react";

import {
  controlPlaneConnection,
  useActiveWorkspaceBinding,
} from "../../controlPlane";
import { NativeReleaseUpdatePrompt } from "../../_shell";
import { isAnyDetentSheetOpen } from "../../_shell/detent-sheet-open";
import { MobileConnectionBanner } from "../MobileConnectionBanner";
import {
  OBSIDIAN_DRAWER_FLICK_PX_PER_MS,
  OBSIDIAN_DRAWER_TRACK_PX,
  obsidianDrawerAbandonsToScroll,
  obsidianDrawerClaimsSwipe,
} from "../../obsidianDrawerGesture";
import {
  expandedSelection,
  hasHorizontalScroller,
  swipeCommits,
} from "../../touchGestures";
import type { Mode as ThemeMode } from "../../theme";
import {
  bindMobileSheetPresentationHold,
  mobilePeekRestLayerSx,
  mobilePresentationMovingRootSx,
  mobileSheetPresentationSx,
} from "../../mobilePresentationMotion";
import { holdStorePresentation } from "../../store";
import { AgentApp } from "../agent/AgentApp";
import {
  drawerProgressOwnsPagerGesture,
  translatedSurfaceOwnsPagerGesture,
} from "../../mobileDrawerDepth";
import {
  MOBILE_DRAWER_SETTLE_EASING,
  mobileDrawerSettleDurationMs,
} from "../../mobileDrawerMotion";
import {
  MOBILE_OPEN_PRODUCT_EVENT,
  mobileProductFromEvent,
  nextMobileProduct,
  pagerDirectionAllowed,
  type PagerGesture,
  pagerOffset,
  pagerTargetOffset,
  shouldReservePagerStart,
  type MobileProduct,
} from "../appPagerMotion";
import { ReviewApp } from "../review/ReviewApp";

const PRODUCT_STORAGE_KEY = "cowboy:mobile-product";
const VELOCITY_COMMIT_PX_PER_MS = OBSIDIAN_DRAWER_FLICK_PX_PER_MS;
const MODAL_OVERLAY_SELECTOR = [
  ".MuiModal-root",
  ".MuiPopover-root",
  ".MuiDialog-root",
  ".MuiDrawer-root",
  "[role='dialog'][aria-modal='true']",
  "[data-mobile-pager-modal='true']",
].join(",");

function restoredProduct(): MobileProduct {
  return globalThis.localStorage?.getItem(PRODUCT_STORAGE_KEY) === "review"
    ? "review"
    : "agent";
}

function ignoredGestureTarget(
  target: EventTarget | null,
  boundary: HTMLElement,
): boolean {
  const element = target instanceof HTMLElement ? target : null;
  const focusedComposer = element?.closest(
    "[data-mobile-focus-composer='true']",
  );
  // Once any editor inside a Composer owns focus, its complete visible card is
  // one writing surface. In particular, the generous blank canvas around a
  // short message must not become a left/right product-navigation handle.
  const focusedComposerOwnsGesture =
    focusedComposer?.matches(":focus-within") === true;
  const explicitlyAllowsPager =
    element?.closest("[data-mobile-pager-allow]") != null;
  return (
    expandedSelection(globalThis.getSelection?.() ?? null) ||
    (!explicitlyAllowsPager &&
      (focusedComposerOwnsGesture ||
        element?.closest(
          "input, textarea, [contenteditable='true'], [data-mobile-pager-ignore]",
        ) != null ||
        hasHorizontalScroller(target, boundary)))
  );
}

function modalOwnsGesture(): boolean {
  return isAnyDetentSheetOpen() ||
    globalThis.document?.querySelector(MODAL_OVERLAY_SELECTOR) != null;
}

function spatialDrawerOwnsGesture(shell: HTMLElement): boolean {
  if (
    shell.querySelector(
      "[data-mobile-drawer-presented='true'], " +
        "[data-mobile-drawer-open='true'], [data-mobile-drawer-moving='true']",
    ) != null
  ) {
    return true;
  }
  const progressed = shell.querySelector("[data-mobile-drawer-progress]");
  if (
    progressed instanceof HTMLElement &&
    drawerProgressOwnsPagerGesture(progressed.getAttribute("data-mobile-drawer-progress"))
  ) {
    return true;
  }
  const surfaces = shell.querySelectorAll("[data-mobile-drawer-surface='true']");
  for (const surface of surfaces) {
    if (
      surface instanceof HTMLElement &&
      translatedSurfaceOwnsPagerGesture(surface.style.transform)
    ) {
      return true;
    }
  }
  return false;
}

export function MobileProductShell({
  themeMode,
  onSetThemeMode,
}: {
  themeMode: ThemeMode;
  onSetThemeMode: (mode: ThemeMode) => void;
}): React.JSX.Element {
  const [product, setProduct] = useState<MobileProduct>(restoredProduct);
  const productRef = useRef(product);
  const agentDrawerOpenRef = useRef(false);
  const reviewDrawerOpenRef = useRef(false);
  const shellRef = useRef<HTMLDivElement>(null);
  const railRef = useRef<HTMLDivElement>(null);
  const agentPageRef = useRef<HTMLDivElement>(null);
  const reviewPageRef = useRef<HTMLDivElement>(null);
  const workspace = useActiveWorkspaceBinding();
  const onAgentDrawerOpenChange = useCallback((open: boolean): void => {
    agentDrawerOpenRef.current = open;
  }, []);
  const onReviewDrawerOpenChange = useCallback((open: boolean): void => {
    reviewDrawerOpenRef.current = open;
  }, []);

  useEffect(() => {
    const shell = shellRef.current;
    const rail = railRef.current;
    const agentPage = agentPageRef.current;
    const reviewPage = reviewPageRef.current;
    if (!shell || !rail || !agentPage || !reviewPage) return undefined;

    let gesture: PagerGesture | null = null;
    let settleTimer = 0;
    let bookkeepingFrame = 0;
    let releaseFrame = 0;
    let releaseIdle: number | undefined;
    let releasePresentation: (() => void) | undefined;
    let directManipulationActive = false;
    let presentationWidth = shell.clientWidth;
    let currentOffset = pagerTargetOffset(productRef.current, presentationWidth);

    const render = (offset: number, width: number): void => {
      currentOffset = offset;
      agentPage.style.transform = `translate3d(${String(offset)}px, 0, 0)`;
      reviewPage.style.transform =
        `translate3d(${String(width + offset)}px, 0, 0)`;
    };
    const armPagerPresentation = (): void => {
      if (directManipulationActive) return;
      releasePresentation ??= holdStorePresentation();
      agentPage.style.willChange = "transform";
      reviewPage.style.willChange = "transform";
    };
    const disarmPagerPresentation = (): void => {
      if (directManipulationActive) return;
      agentPage.style.removeProperty("will-change");
      reviewPage.style.removeProperty("will-change");
      releasePresentation?.();
      releasePresentation = undefined;
    };
    const beginDirectManipulation = (): void => {
      if (directManipulationActive) return;
      if (releaseFrame !== 0) globalThis.cancelAnimationFrame(releaseFrame);
      if (releaseIdle !== undefined) {
        if (typeof globalThis.cancelIdleCallback === "function") {
          globalThis.cancelIdleCallback(releaseIdle);
        } else {
          globalThis.clearTimeout(releaseIdle);
        }
        releaseIdle = undefined;
      }
      armPagerPresentation();
      directManipulationActive = true;
      shell.setAttribute("data-mobile-product-moving", "true");
      globalThis.dispatchEvent(
        new CustomEvent("cowboy:transcript-direct-manipulation-start"),
      );
    };
    const releaseDirectManipulation = (): void => {
      const finish = (): void => {
        releaseIdle = undefined;
        directManipulationActive = false;
        shell.removeAttribute("data-mobile-product-moving");
        releasePresentation?.();
        releasePresentation = undefined;
        globalThis.dispatchEvent(
          new CustomEvent("cowboy:transcript-direct-manipulation-end"),
        );
      };
      releaseFrame = globalThis.requestAnimationFrame(() => {
        releaseFrame = 0;
        if (typeof globalThis.requestIdleCallback === "function") {
          releaseIdle = globalThis.requestIdleCallback(finish, { timeout: 180 });
        } else {
          releaseIdle = globalThis.setTimeout(finish, 32);
        }
      });
    };
    const settle = (
      next: MobileProduct,
      releaseVelocity = 0,
      cachedWidth?: number,
    ): void => {
      globalThis.clearTimeout(settleTimer);
      const releaseOffset = currentOffset;
      // Complete geometry reads before transition/style writes. Both pages are
      // large application surfaces, so a forced layout here is expensive.
      const width = cachedWidth ?? shell.clientWidth;
      presentationWidth = width;
      const targetOffset = pagerTargetOffset(next, width);
      const remaining = Math.min(
        1,
        Math.abs(targetOffset - releaseOffset) / width,
      );
      const duration = mobileDrawerSettleDurationMs(remaining, releaseVelocity);
      const transition =
        `transform ${String(duration)}ms ${MOBILE_DRAWER_SETTLE_EASING}`;
      agentPage.style.transition = transition;
      reviewPage.style.transition = transition;
      render(targetOffset, width);
      productRef.current = next;
      globalThis.localStorage?.setItem(PRODUCT_STORAGE_KEY, next);
      settleTimer = globalThis.setTimeout(() => {
        agentPage.style.removeProperty("transition");
        reviewPage.style.removeProperty("transition");
        setProduct(next);
        if (directManipulationActive) releaseDirectManipulation();
      }, duration + 20);
    };
    const onTouchStart = (event: TouchEvent): void => {
      const touch = event.touches[0];
      if (!touch) return;
      const ignored = ignoredGestureTarget(event.target, shell);
      const overlayOwnsGesture =
        modalOwnsGesture() ||
        spatialDrawerOwnsGesture(shell) ||
        (productRef.current === "agent" && agentDrawerOpenRef.current) ||
        (productRef.current === "review" && reviewDrawerOpenRef.current);
      if (!shouldReservePagerStart(ignored, overlayOwnsGesture)) {
        gesture = null;
        return;
      }
      const now = performance.now();
      const width = shell.clientWidth;
      presentationWidth = width;
      gesture = {
        product: productRef.current,
        width,
        startX: touch.clientX,
        startY: touch.clientY,
        lastX: touch.clientX,
        lastAt: now,
        velocity: 0,
        locked: false,
      };
      // Promote both pages before the 2 px claim so the first translate
      // only writes transform. Overflow flatten waits for rAF bookkeeping.
      armPagerPresentation();
    };
    const onTouchMove = (event: TouchEvent): void => {
      const touch = event.touches[0];
      if (!gesture || !touch) return;
      if (expandedSelection(globalThis.getSelection?.() ?? null)) {
        if (gesture.locked) settle(gesture.product, 0, gesture.width);
        gesture = null;
        return;
      }
      // Overlay ownership is decided on touchstart. Querying the sheet/drawer
      // tree on every vertical sample keeps Agent/Code scrolling on the main
      // thread. Recheck only after this pager already owns the gesture.
      if (
        gesture.locked &&
        (modalOwnsGesture() ||
          spatialDrawerOwnsGesture(shell) ||
          (productRef.current === "agent" && agentDrawerOpenRef.current) ||
          (productRef.current === "review" && reviewDrawerOpenRef.current))
      ) {
        settle(gesture.product, 0, gesture.width);
        gesture = null;
        return;
      }
      const deltaX = touch.clientX - gesture.startX;
      const deltaY = touch.clientY - gesture.startY;
      if (!gesture.locked && obsidianDrawerAbandonsToScroll(deltaX, deltaY)) {
        disarmPagerPresentation();
        gesture = null;
        // The spatial drawer is a descendant and has its own non-passive
        // horizontal recognizer. Keep this stream's native vertical scroll
        // default, but do not let the drawer reinterpret a diagonal sample as
        // a horizontal swipe and call preventDefault().
        event.stopPropagation();
        return;
      }
      const swipe = gesture.locked
        ? {
          direction: deltaX < 0 ? "left" as const : "right" as const,
          distance: Math.abs(deltaX),
        }
        : obsidianDrawerClaimsSwipe(deltaX, deltaY, OBSIDIAN_DRAWER_TRACK_PX);
      if (!swipe || !pagerDirectionAllowed(gesture.product, deltaX)) return;
      if (!gesture.locked) {
        gesture.locked = true;
        agentPage.style.transition = "none";
        reviewPage.style.transition = "none";
      }
      event.preventDefault();
      event.stopPropagation();
      const now = performance.now();
      const elapsed = Math.max(1, now - gesture.lastAt);
      const velocity = (touch.clientX - gesture.lastX) / elapsed;
      gesture.velocity = gesture.velocity * 0.65 + velocity * 0.35;
      gesture.lastX = touch.clientX;
      gesture.lastAt = now;
      render(
        pagerOffset(gesture.product, deltaX, gesture.width),
        gesture.width,
      );
      if (bookkeepingFrame === 0 && !directManipulationActive) {
        bookkeepingFrame = globalThis.requestAnimationFrame(() => {
          bookkeepingFrame = 0;
          beginDirectManipulation();
        });
      }
    };
    const onTouchEnd = (): void => {
      if (!gesture) return;
      if (!gesture.locked) {
        gesture = null;
        disarmPagerPresentation();
        return;
      }
      const distance = Math.abs(gesture.lastX - gesture.startX);
      const velocityCommits = gesture.product === "agent"
        ? gesture.velocity <= -VELOCITY_COMMIT_PX_PER_MS
        : gesture.velocity >= VELOCITY_COMMIT_PX_PER_MS;
      const next = velocityCommits || swipeCommits(distance, gesture.width)
        ? nextMobileProduct(gesture.product)
        : gesture.product;
      const velocity = gesture.velocity;
      const width = gesture.width;
      gesture = null;
      settle(next, velocity, width);
    };
    const onTouchCancel = (): void => {
      const locked = gesture?.locked === true;
      const current = gesture?.product ?? productRef.current;
      const width = gesture?.width;
      gesture = null;
      if (!locked) {
        disarmPagerPresentation();
        return;
      }
      settle(current, 0, width);
    };
    const onResize = (): void => {
      const width = shell.clientWidth;
      presentationWidth = width;
      render(pagerTargetOffset(productRef.current, width), width);
    };

    const onOpenProduct = (event: Event): void => {
      const next = mobileProductFromEvent(event);
      if (next === null || next === productRef.current) return;
      settle(next);
    };

    render(currentOffset, presentationWidth);
    globalThis.addEventListener(MOBILE_OPEN_PRODUCT_EVENT, onOpenProduct);
    shell.addEventListener("touchstart", onTouchStart, {
      capture: true,
      passive: true,
    });
    shell.addEventListener("touchmove", onTouchMove, {
      capture: true,
      passive: false,
    });
    shell.addEventListener("touchend", onTouchEnd, {
      capture: true,
      passive: true,
    });
    shell.addEventListener("touchcancel", onTouchCancel, {
      capture: true,
      passive: true,
    });
    globalThis.addEventListener("resize", onResize);
    const releaseSheetHold = bindMobileSheetPresentationHold(shell);
    return () => {
      releaseSheetHold();
      globalThis.removeEventListener(MOBILE_OPEN_PRODUCT_EVENT, onOpenProduct);
      shell.removeEventListener("touchstart", onTouchStart, true);
      shell.removeEventListener("touchmove", onTouchMove, true);
      shell.removeEventListener("touchend", onTouchEnd, true);
      shell.removeEventListener("touchcancel", onTouchCancel, true);
      globalThis.removeEventListener("resize", onResize);
      globalThis.clearTimeout(settleTimer);
      if (bookkeepingFrame !== 0) globalThis.cancelAnimationFrame(bookkeepingFrame);
      if (releaseFrame !== 0) globalThis.cancelAnimationFrame(releaseFrame);
      if (releaseIdle !== undefined) {
        if (typeof globalThis.cancelIdleCallback === "function") {
          globalThis.cancelIdleCallback(releaseIdle);
        } else {
          globalThis.clearTimeout(releaseIdle);
        }
      }
      releasePresentation?.();
      shell.removeAttribute("data-mobile-product-moving");
      if (directManipulationActive) {
        globalThis.dispatchEvent(
          new CustomEvent("cowboy:transcript-direct-manipulation-end"),
        );
      }
    };
  }, []);

  return (
    <Box
      ref={shellRef}
      data-mobile-product={product}
      sx={{
        width: "100%",
        height: "100%",
        overflow: "hidden",
        position: "relative",
        bgcolor: "background.default",
        ...mobilePeekRestLayerSx,
        ...mobilePresentationMovingRootSx("data-mobile-product-moving"),
        ...mobileSheetPresentationSx,
      }}
    >
      <MobileConnectionBanner store={controlPlaneConnection} />
      <NativeReleaseUpdatePrompt
        appId="top.thundersparrow.cowboy"
        manifestUrl="/native-release.json"
      />
      <Box
        ref={railRef}
        data-workspace-session={workspace?.sessionId}
        data-workspace-cwd={workspace?.cwd}
        sx={{
          width: "100%",
          height: "100%",
          position: "relative",
          backfaceVisibility: "hidden",
        }}
      >
        <Box
          ref={agentPageRef}
          aria-hidden={product !== "agent"}
          inert={product !== "agent"}
          sx={{
            position: "absolute",
            inset: 0,
            width: "100%",
            height: "100%",
            minWidth: 0,
            overflow: "hidden",
            backfaceVisibility: "hidden",
            WebkitBackfaceVisibility: "hidden",
            contain: "layout paint style",
            isolation: "isolate",
            transform: product === "agent"
              ? "translate3d(0, 0, 0)"
              : "translate3d(-100%, 0, 0)",
          }}
        >
          <AgentApp
            themeMode={themeMode}
            onSetThemeMode={onSetThemeMode}
            onDrawerOpenChange={onAgentDrawerOpenChange}
          />
        </Box>
        <Box
          ref={reviewPageRef}
          aria-hidden={product !== "review"}
          inert={product !== "review"}
          sx={{
            position: "absolute",
            inset: 0,
            width: "100%",
            height: "100%",
            minWidth: 0,
            overflow: "hidden",
            backfaceVisibility: "hidden",
            WebkitBackfaceVisibility: "hidden",
            // Paint containment folds CodeMirror into this page tile and
            // re-rasters it on every pager frame. Keep layout isolation only.
            contain: "layout style",
            isolation: "isolate",
            transform: product === "review"
              ? "translate3d(0, 0, 0)"
              : "translate3d(100%, 0, 0)",
          }}
        >
          <ReviewApp
            active={product === "review"}
            onDrawerOpenChange={onReviewDrawerOpenChange}
          />
        </Box>
      </Box>
    </Box>
  );
}
