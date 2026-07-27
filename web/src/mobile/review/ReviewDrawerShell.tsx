import { Box } from "@mui/material";
import { useEffect, useRef, useState } from "react";
import { navigationHaptic, prepareNavigationHaptic } from "../../haptic";
import {
  mobileDrawerSurfaceVisual,
  predictDrawerOffset,
} from "../../mobileDrawerMotion";
import {
  expandedSelection,
  hasHorizontalScroller,
  horizontalSwipe,
  swipeCommits,
} from "../../touchGestures";

const VELOCITY_COMMIT_PX_PER_MS = 0.45;

export function ReviewDrawerShell({
  drawer,
  children,
  onOpenChange,
  closeRequest = 0,
  toggleRequest = 0,
}: {
  drawer: React.ReactNode;
  children: React.ReactNode;
  onOpenChange: (open: boolean) => void;
  closeRequest?: number;
  toggleRequest?: number;
}): React.JSX.Element {
  const rootRef = useRef<HTMLDivElement>(null);
  const drawerRef = useRef<HTMLDivElement>(null);
  const maskRef = useRef<HTMLDivElement>(null);
  const surfaceRef = useRef<HTMLDivElement>(null);
  const closeRef = useRef<() => void>(() => undefined);
  const toggleRef = useRef<() => void>(() => undefined);
  const openRef = useRef(false);
  const [open, setOpen] = useState(false);

  useEffect(() => {
    if (closeRequest > 0) closeRef.current();
  }, [closeRequest]);

  useEffect(() => {
    if (toggleRequest > 0) toggleRef.current();
  }, [toggleRequest]);

  useEffect(() => {
    const root = rootRef.current;
    const drawerElement = drawerRef.current;
    const mask = maskRef.current;
    const surface = surfaceRef.current;
    if (!root || !drawerElement || !mask || !surface) return undefined;

    let gesture: {
      startX: number;
      startY: number;
      lastX: number;
      lastAt: number;
      velocity: number;
      locked: boolean;
      base: number;
    } | null = null;
    let frame = 0;
    let pendingOffset = 0;
    let pendingAt = 0;
    let pendingVelocity = 0;

    const width = (): number =>
      root.clientWidth < 768
        ? Math.min(360, root.clientWidth * 0.84)
        : Math.min(440, root.clientWidth * 0.52);
    const render = (offset: number): void => {
      const drawerWidth = width();
      const clamped = Math.max(0, Math.min(drawerWidth, offset));
      const phone = root.clientWidth < 768;
      const visual = mobileDrawerSurfaceVisual(clamped, drawerWidth, phone);
      const parallax = drawerWidth * (phone ? 0.28 : 0.22) *
        (1 - visual.progress);
      surface.style.transform = `translate3d(-${
        String(clamped)
      }px, 0, 0) scale(${String(visual.scale)})`;
      surface.style.opacity = String(visual.opacity);
      drawerElement.style.transform = `translate3d(${
        String(parallax)
      }px, 0, 0)`;
      drawerElement.style.opacity = String(0.72 + visual.progress * 0.28);
      mask.style.transform = `translate3d(-${String(clamped)}px, 0, 0)`;
    };
    const scheduleRender = (
      offset: number,
      sampleAt: number,
      velocity: number,
    ): void => {
      pendingOffset = offset;
      pendingAt = sampleAt;
      pendingVelocity = velocity;
      if (frame !== 0) return;
      frame = requestAnimationFrame((frameAt) => {
        frame = 0;
        render(
          predictDrawerOffset(
            pendingOffset,
            -pendingVelocity,
            frameAt - pendingAt,
          ),
        );
      });
    };
    const settle = (nextOpen: boolean, velocity = 0): void => {
      if (frame !== 0) cancelAnimationFrame(frame);
      frame = 0;
      const drawerWidth = width();
      const duration = Math.max(
        150,
        Math.min(250, 225 - Math.min(65, Math.abs(velocity) * 45)),
      );
      for (const element of [surface, drawerElement, mask]) {
        element.style.transition = `transform ${
          String(duration)
        }ms cubic-bezier(0.22, 1, 0.36, 1), opacity ${String(duration)}ms ease`;
      }
      surface.style.borderRadius = nextOpen
        ? `${String(root.clientWidth < 768 ? 36 : 30)}px`
        : "0";
      surface.style.boxShadow = nextOpen
        ? "18px 0 42px rgba(0,0,0,0.16)"
        : "none";
      render(nextOpen ? drawerWidth : 0);
      const changed = nextOpen !== openRef.current;
      openRef.current = nextOpen;
      setOpen(nextOpen);
      onOpenChange(nextOpen);
      if (changed) navigationHaptic();
      globalThis.setTimeout(() => {
        for (const element of [surface, drawerElement, mask]) {
          element.style.removeProperty("transition");
        }
      }, duration + 20);
    };
    closeRef.current = () => settle(false);
    toggleRef.current = () => settle(!openRef.current);

    const onTouchStart = (event: TouchEvent): void => {
      const touch = event.touches[0];
      const target = event.target;
      if (
        !touch ||
        expandedSelection(globalThis.getSelection?.() ?? null) ||
        (target instanceof HTMLElement &&
          target.closest("input, textarea, [contenteditable='true']")) ||
        hasHorizontalScroller(target, root)
      ) {
        gesture = null;
        return;
      }
      const now = performance.now();
      gesture = {
        startX: touch.clientX,
        startY: touch.clientY,
        lastX: touch.clientX,
        lastAt: now,
        velocity: 0,
        locked: false,
        base: openRef.current ? width() : 0,
      };
      prepareNavigationHaptic();
    };
    const onTouchMove = (event: TouchEvent): void => {
      const touch = event.touches[0];
      if (!gesture || !touch) return;
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
      const directionAllowed = openRef.current ? deltaX > 0 : deltaX < 0;
      if (!swipe || !directionAllowed) return;
      if (!gesture.locked) {
        gesture.locked = true;
        const radius = root.clientWidth < 768 ? 36 : 30;
        surface.style.borderRadius = `${String(radius)}px`;
        surface.style.boxShadow = "18px 0 42px rgba(0,0,0,0.16)";
        for (const element of [surface, drawerElement, mask]) {
          element.style.transition = "none";
          element.style.willChange = "transform, opacity";
        }
      }
      event.preventDefault();
      event.stopPropagation();
      const now = performance.now();
      const elapsed = Math.max(1, now - gesture.lastAt);
      const velocity = (touch.clientX - gesture.lastX) / elapsed;
      gesture.velocity = gesture.velocity * 0.65 + velocity * 0.35;
      gesture.lastX = touch.clientX;
      gesture.lastAt = now;
      scheduleRender(gesture.base - deltaX, now, gesture.velocity);
    };
    const onTouchEnd = (): void => {
      if (!gesture?.locked) {
        gesture = null;
        return;
      }
      const distance = Math.abs(gesture.lastX - gesture.startX);
      const velocityCommits = openRef.current
        ? gesture.velocity >= VELOCITY_COMMIT_PX_PER_MS
        : gesture.velocity <= -VELOCITY_COMMIT_PX_PER_MS;
      const nextOpen = velocityCommits || swipeCommits(distance, width())
        ? !openRef.current
        : openRef.current;
      const velocity = gesture.velocity;
      gesture = null;
      settle(nextOpen, velocity);
    };
    const onTouchCancel = (): void => {
      gesture = null;
      settle(openRef.current);
    };
    const onResize = (): void => render(openRef.current ? width() : 0);

    render(0);
    root.addEventListener("touchstart", onTouchStart, { passive: true });
    root.addEventListener("touchmove", onTouchMove, { passive: false });
    root.addEventListener("touchend", onTouchEnd, { passive: true });
    root.addEventListener("touchcancel", onTouchCancel, { passive: true });
    globalThis.addEventListener("resize", onResize);
    return () => {
      root.removeEventListener("touchstart", onTouchStart);
      root.removeEventListener("touchmove", onTouchMove);
      root.removeEventListener("touchend", onTouchEnd);
      root.removeEventListener("touchcancel", onTouchCancel);
      globalThis.removeEventListener("resize", onResize);
      if (frame !== 0) cancelAnimationFrame(frame);
      closeRef.current = () => undefined;
      toggleRef.current = () => undefined;
    };
  }, [onOpenChange]);

  return (
    <Box ref={rootRef} sx={{ position: "relative", width: 1, height: 1 }}>
      <Box
        ref={drawerRef}
        aria-hidden={!open}
        sx={{
          position: "absolute",
          zIndex: 0,
          inset: 0,
          pl: "calc(100% - min(84%, 360px))",
          bgcolor: "background.default",
          backfaceVisibility: "hidden",
          "@media (min-width: 768px)": {
            pl: "calc(100% - min(52%, 440px))",
          },
        }}
      >
        {drawer}
      </Box>
      <Box
        ref={maskRef}
        aria-hidden="true"
        sx={{
          position: "absolute",
          zIndex: 0,
          inset: 0,
          bgcolor: "background.default",
          pointerEvents: "none",
          backfaceVisibility: "hidden",
        }}
      />
      <Box
        ref={surfaceRef}
        sx={{
          position: "absolute",
          zIndex: 1,
          inset: 0,
          overflow: "hidden",
          bgcolor: "background.default",
          backfaceVisibility: "hidden",
          transformOrigin: "left center",
        }}
      >
        {open && (
          <Box
            aria-label="Close worktree drawer"
            onClick={() => closeRef.current()}
            sx={{ position: "absolute", zIndex: 2, inset: 0 }}
          />
        )}
        {children}
      </Box>
    </Box>
  );
}
