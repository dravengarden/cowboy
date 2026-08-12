import { Box } from "@mui/material";
import { useCallback, useEffect, useRef, useState } from "react";
import { navigationHaptic, prepareNavigationHaptic } from "../../haptic";
import {
  controlPlaneConnection,
  useActiveWorkspaceBinding,
} from "../../controlPlane";
import { NativeReleaseUpdatePrompt } from "../../_shell";
import { isAnyDetentSheetOpen } from "../../_shell/detent-sheet-open";
import { MobileConnectionBanner } from "../MobileConnectionBanner";
import {
  expandedSelection,
  hasHorizontalScroller,
  horizontalSwipe,
  isDominantVerticalPan,
  swipeCommits,
} from "../../touchGestures";
import type { Mode as ThemeMode } from "../../theme";
import { holdStorePresentation } from "../../store";
import { AgentApp } from "../agent/AgentApp";
import {
  nextMobileProduct,
  pagerDirectionAllowed,
  type PagerGesture,
  pagerOffset,
  pagerTargetOffset,
  predictPagerOffset,
  shouldReservePagerStart,
  type MobileProduct,
} from "../appPagerMotion";
import { ReviewApp } from "../review/ReviewApp";

const PRODUCT_STORAGE_KEY = "cowboy:mobile-product";
const VELOCITY_COMMIT_PX_PER_MS = 0.45;
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
  return shell.querySelector(
    "[data-mobile-drawer-presented='true'], " +
      "[data-mobile-drawer-open='true'], [data-mobile-drawer-moving='true']",
  ) != null;
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
    let frame = 0;
    let settleTimer = 0;
    let releaseFrame = 0;
    let releaseIdle: number | undefined;
    let releasePresentation: (() => void) | undefined;
    let directManipulationActive = false;
    let presentationWidth = shell.clientWidth;
    let currentOffset = pagerTargetOffset(productRef.current, presentationWidth);
    let pendingOffset = currentOffset;
    let pendingSampleAt = 0;
    let pendingVelocity = 0;

    const render = (offset: number, width: number): void => {
      currentOffset = offset;
      agentPage.style.transform = `translate3d(${String(offset)}px, 0, 0)`;
      reviewPage.style.transform =
        `translate3d(${String(width + offset)}px, 0, 0)`;
    };
    const scheduleRender = (
      offset: number,
      sampleAt: number,
      velocity: number,
    ): void => {
      pendingOffset = offset;
      pendingSampleAt = sampleAt;
      pendingVelocity = velocity;
      if (frame !== 0) return;
      frame = globalThis.requestAnimationFrame((frameAt) => {
        frame = 0;
        render(
          predictPagerOffset(
            pendingOffset,
            pendingVelocity,
            frameAt - pendingSampleAt,
            presentationWidth,
          ),
          presentationWidth,
        );
      });
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
      const releaseOffset = frame !== 0 ? pendingOffset : currentOffset;
      if (frame !== 0) globalThis.cancelAnimationFrame(frame);
      frame = 0;
      // Complete geometry reads before transition/style writes. Both pages are
      // large application surfaces, so a forced layout here is expensive.
      const width = cachedWidth ?? shell.clientWidth;
      presentationWidth = width;
      const targetOffset = pagerTargetOffset(next, width);
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
      const transition =
        `transform ${String(duration)}ms cubic-bezier(0.22, 1, 0.36, 1)`;
      agentPage.style.transition = transition;
      reviewPage.style.transition = transition;
      render(targetOffset, width);
      const changed = next !== productRef.current;
      productRef.current = next;
      globalThis.localStorage?.setItem(PRODUCT_STORAGE_KEY, next);
      if (changed) navigationHaptic();
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
      prepareNavigationHaptic();
    };
    const onTouchMove = (event: TouchEvent): void => {
      const touch = event.touches[0];
      if (!gesture || !touch) return;
      if (
        expandedSelection(globalThis.getSelection?.() ?? null) ||
        modalOwnsGesture() ||
        spatialDrawerOwnsGesture(shell) ||
        (productRef.current === "agent" && agentDrawerOpenRef.current) ||
        (productRef.current === "review" && reviewDrawerOpenRef.current)
      ) {
        if (gesture.locked) settle(gesture.product, 0, gesture.width);
        gesture = null;
        return;
      }
      const deltaX = touch.clientX - gesture.startX;
      const deltaY = touch.clientY - gesture.startY;
      if (!gesture.locked && isDominantVerticalPan(deltaX, deltaY)) {
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
        : horizontalSwipe(deltaX, deltaY, 8, 1.15);
      if (!swipe || !pagerDirectionAllowed(gesture.product, deltaX)) return;
      if (!gesture.locked) {
        if (releaseFrame !== 0) globalThis.cancelAnimationFrame(releaseFrame);
        if (releaseIdle !== undefined) {
          if (typeof globalThis.cancelIdleCallback === "function") {
            globalThis.cancelIdleCallback(releaseIdle);
          } else {
            globalThis.clearTimeout(releaseIdle);
          }
          releaseIdle = undefined;
        }
        gesture.locked = true;
        releasePresentation ??= holdStorePresentation();
        directManipulationActive = true;
        shell.setAttribute("data-mobile-product-moving", "true");
        globalThis.dispatchEvent(
          new CustomEvent("cowboy:transcript-direct-manipulation-start"),
        );
        agentPage.style.transition = "none";
        reviewPage.style.transition = "none";
        agentPage.style.willChange = "transform";
        reviewPage.style.willChange = "transform";
      }
      event.preventDefault();
      event.stopPropagation();
      const now = performance.now();
      const elapsed = Math.max(1, now - gesture.lastAt);
      const velocity = (touch.clientX - gesture.lastX) / elapsed;
      gesture.velocity = gesture.velocity * 0.65 + velocity * 0.35;
      gesture.lastX = touch.clientX;
      gesture.lastAt = now;
      scheduleRender(
        pagerOffset(gesture.product, deltaX, gesture.width),
        now,
        gesture.velocity,
      );
    };
    const onTouchEnd = (): void => {
      if (!gesture) return;
      if (!gesture.locked) {
        gesture = null;
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
      const current = gesture?.product ?? productRef.current;
      const width = gesture?.width;
      gesture = null;
      settle(current, 0, width);
    };
    const onResize = (): void => {
      const width = shell.clientWidth;
      presentationWidth = width;
      render(pagerTargetOffset(productRef.current, width), width);
    };

    render(currentOffset, presentationWidth);
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
    return () => {
      shell.removeEventListener("touchstart", onTouchStart, true);
      shell.removeEventListener("touchmove", onTouchMove, true);
      shell.removeEventListener("touchend", onTouchEnd, true);
      shell.removeEventListener("touchcancel", onTouchCancel, true);
      globalThis.removeEventListener("resize", onResize);
      globalThis.clearTimeout(settleTimer);
      if (frame !== 0) globalThis.cancelAnimationFrame(frame);
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
        "&[data-mobile-product-moving='true'] *": {
          animationPlayState: "paused !important",
        },
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
            willChange: "transform",
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
            contain: "layout paint style",
            isolation: "isolate",
            willChange: "transform",
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
