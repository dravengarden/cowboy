import { Box } from "@mui/material";
import { useCallback, useEffect, useRef, useState } from "react";
import { navigationHaptic, prepareNavigationHaptic } from "../../haptic";
import {
  controlPlaneConnection,
  useActiveWorkspaceBinding,
} from "../../controlPlane";
import { NativeReleaseUpdatePrompt } from "../../_shell";
import { MobileConnectionBanner } from "../MobileConnectionBanner";
import {
  expandedSelection,
  hasHorizontalScroller,
  horizontalSwipe,
  swipeCommits,
} from "../../touchGestures";
import type { Mode as ThemeMode } from "../../theme";
import { AgentApp } from "../agent/AgentApp";
import {
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
const VELOCITY_COMMIT_PX_PER_MS = 0.45;

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
  const explicitlyAllowsPager =
    element?.closest("[data-mobile-pager-allow]") != null;
  return (
    expandedSelection(globalThis.getSelection?.() ?? null) ||
    (!explicitlyAllowsPager &&
      (element?.closest(
          "input, textarea, [contenteditable='true'], [data-mobile-pager-ignore]",
        ) != null ||
        hasHorizontalScroller(target, boundary)))
  );
}

export function MobileProductShell({
  themeMode,
  onSetThemeMode,
}: {
  themeMode: ThemeMode;
  onSetThemeMode: (mode: ThemeMode) => void;
}): React.JSX.Element {
  const [product, setProduct] = useState<MobileProduct>(restoredProduct);
  const [agentDrawerOpen, setAgentDrawerOpen] = useState(false);
  const [reviewDrawerOpen, setReviewDrawerOpen] = useState(false);
  const productRef = useRef(product);
  const shellRef = useRef<HTMLDivElement>(null);
  const railRef = useRef<HTMLDivElement>(null);
  const agentPageRef = useRef<HTMLDivElement>(null);
  const reviewPageRef = useRef<HTMLDivElement>(null);
  const settleTimerRef = useRef<number | undefined>(undefined);
  const workspace = useActiveWorkspaceBinding();

  const selectProduct = useCallback((next: MobileProduct): void => {
    const shell = shellRef.current;
    productRef.current = next;
    globalThis.localStorage?.setItem(PRODUCT_STORAGE_KEY, next);
    if (!shell) {
      setProduct(next);
      return;
    }
    if (settleTimerRef.current !== undefined) {
      globalThis.clearTimeout(settleTimerRef.current);
    }
    const pages = [agentPageRef.current, reviewPageRef.current];
    for (const page of pages) {
      if (!page) continue;
      page.style.transition = "transform 220ms cubic-bezier(0.22, 1, 0.36, 1)";
    }
    const offset = pagerTargetOffset(next, shell.clientWidth);
    if (agentPageRef.current) {
      agentPageRef.current.style.transform =
        `translate3d(${String(offset)}px, 0, 0)`;
    }
    if (reviewPageRef.current) {
      reviewPageRef.current.style.transform =
        `translate3d(${String(shell.clientWidth + offset)}px, 0, 0)`;
    }
    settleTimerRef.current = globalThis.setTimeout(() => {
      for (const page of pages) page?.style.removeProperty("transition");
      settleTimerRef.current = undefined;
      // Keep the settle animation compositor-only. Updating `product` changes
      // inert/aria state and starts or pauses Review data work, so committing
      // it on the first animation frame makes iPad WebKit repaint both large
      // application surfaces while they are moving.
      setProduct(next);
    }, 240);
  }, []);

  useEffect(() =>
    () => {
      if (settleTimerRef.current !== undefined) {
        globalThis.clearTimeout(settleTimerRef.current);
      }
    }, []);

  useEffect(() => {
    const shell = shellRef.current;
    const rail = railRef.current;
    const agentPage = agentPageRef.current;
    const reviewPage = reviewPageRef.current;
    if (!shell || !rail || !agentPage || !reviewPage) return undefined;

    let gesture: PagerGesture | null = null;
    let frame = 0;
    let pendingOffset = pagerTargetOffset(productRef.current, shell.clientWidth);

    const render = (offset: number): void => {
      agentPage.style.transform = `translate3d(${String(offset)}px, 0, 0)`;
      reviewPage.style.transform =
        `translate3d(${String(shell.clientWidth + offset)}px, 0, 0)`;
    };
    const scheduleRender = (offset: number): void => {
      pendingOffset = offset;
      if (frame !== 0) return;
      frame = globalThis.requestAnimationFrame(() => {
        frame = 0;
        render(pendingOffset);
      });
    };
    const settle = (next: MobileProduct): void => {
      if (frame !== 0) globalThis.cancelAnimationFrame(frame);
      frame = 0;
      const changed = next !== productRef.current;
      selectProduct(next);
      if (changed) navigationHaptic();
    };
    const onTouchStart = (event: TouchEvent): void => {
      const touch = event.touches[0];
      if (!touch) return;
      const ignored = ignoredGestureTarget(event.target, shell);
      const overlayOwnsGesture =
        (productRef.current === "agent" && agentDrawerOpen) ||
        (productRef.current === "review" && reviewDrawerOpen);
      if (!shouldReservePagerStart(ignored, overlayOwnsGesture)) {
        gesture = null;
        return;
      }
      const now = performance.now();
      gesture = {
        product: productRef.current,
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
      if (expandedSelection(globalThis.getSelection?.() ?? null)) {
        gesture = null;
        return;
      }
      const deltaX = touch.clientX - gesture.startX;
      const deltaY = touch.clientY - gesture.startY;
      if (
        !gesture.locked && Math.abs(deltaY) >= 10 &&
        Math.abs(deltaY) > Math.abs(deltaX) * 1.15
      ) {
        gesture = null;
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
        gesture.locked = true;
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
      scheduleRender(pagerOffset(gesture.product, deltaX, shell.clientWidth));
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
      const next = velocityCommits || swipeCommits(distance, shell.clientWidth)
        ? nextMobileProduct(gesture.product)
        : gesture.product;
      gesture = null;
      settle(next);
    };
    const onTouchCancel = (): void => {
      const current = gesture?.product ?? productRef.current;
      gesture = null;
      settle(current);
    };
    const onResize = (): void => render(
      pagerTargetOffset(productRef.current, shell.clientWidth),
    );

    render(pagerTargetOffset(productRef.current, shell.clientWidth));
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
      if (frame !== 0) globalThis.cancelAnimationFrame(frame);
    };
  }, [agentDrawerOpen, reviewDrawerOpen, selectProduct]);

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
            onDrawerOpenChange={setAgentDrawerOpen}
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
            onDrawerOpenChange={setReviewDrawerOpen}
          />
        </Box>
      </Box>
    </Box>
  );
}
