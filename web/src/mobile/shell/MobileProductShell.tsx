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
  return (
    expandedSelection(globalThis.getSelection?.() ?? null) ||
    element?.closest(
        "input, textarea, [contenteditable='true'], [data-mobile-pager-ignore]",
      ) != null ||
    hasHorizontalScroller(target, boundary)
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
  const productRef = useRef(product);
  const shellRef = useRef<HTMLDivElement>(null);
  const railRef = useRef<HTMLDivElement>(null);
  const workspace = useActiveWorkspaceBinding();

  const selectProduct = useCallback((next: MobileProduct): void => {
    const shell = shellRef.current;
    const rail = railRef.current;
    productRef.current = next;
    setProduct(next);
    globalThis.localStorage?.setItem(PRODUCT_STORAGE_KEY, next);
    if (!shell || !rail) return;
    rail.style.transition = "transform 220ms cubic-bezier(0.22, 1, 0.36, 1)";
    rail.style.transform =
      `translate3d(${String(pagerTargetOffset(next, shell.clientWidth))}px, 0, 0)`;
    globalThis.setTimeout(() => rail.style.removeProperty("transition"), 240);
  }, []);

  useEffect(() => {
    const shell = shellRef.current;
    const rail = railRef.current;
    if (!shell || !rail) return undefined;

    let gesture: PagerGesture | null = null;
    let frame = 0;
    let pendingOffset = pagerTargetOffset(productRef.current, shell.clientWidth);

    const render = (offset: number): void => {
      rail.style.transform = `translate3d(${String(offset)}px, 0, 0)`;
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
      if (!shouldReservePagerStart(ignored)) {
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
        rail.style.transition = "none";
        rail.style.willChange = "transform";
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
  }, [selectProduct]);

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
          display: "flex",
          width: "200%",
          height: "100%",
          backfaceVisibility: "hidden",
        }}
      >
        <Box
          aria-hidden={product !== "agent"}
          inert={product !== "agent"}
          sx={{ width: "50%", height: "100%", minWidth: 0, overflow: "hidden" }}
        >
          <AgentApp
            themeMode={themeMode}
            onSetThemeMode={onSetThemeMode}
          />
        </Box>
        <Box
          aria-hidden={product !== "review"}
          inert={product !== "review"}
          sx={{ width: "50%", height: "100%", minWidth: 0, overflow: "hidden" }}
        >
          <ReviewApp
            themeMode={themeMode}
            onSetThemeMode={onSetThemeMode}
          />
        </Box>
      </Box>
    </Box>
  );
}
