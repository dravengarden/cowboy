import { useCallback, useRef } from "react";
import type {
  MouseEvent as ReactMouseEvent,
  PointerEvent as ReactPointerEvent,
} from "react";
import {
  createRetargetedTouchClickGuard,
  isPairedTouchClick,
  RELIABLE_TOUCH_TAP_MOVE_SLOP_PX,
} from "./touchGestures";

const MAX_TAP_MS = 800;

interface TouchStart {
  pointerId: number;
  x: number;
  y: number;
  at: number;
}

function isNestedControl(target: EventTarget | null, current: HTMLElement): boolean {
  if (!(target instanceof Element)) return false;
  const control = target.closest(
    "button, a, input, select, textarea, [role='button'], [data-pending-content-action]",
  );
  return control !== null && control !== current;
}

/**
 * Preserve normal click/keyboard activation while committing a stationary touch
 * on pointerup. Mobile Safari can omit the later synthetic click when the touch
 * first stops momentum in a scroll container; pointerup is still delivered.
 * A movement threshold keeps scrolling and drag gestures entirely native.
 */
export function useReliableTouchTap<T extends HTMLElement>(
  onActivate: () => void,
  beforeTouchActivate?: () => void,
): {
  onPointerDown: (event: ReactPointerEvent<T>) => void;
  onPointerMove: (event: ReactPointerEvent<T>) => void;
  onPointerUp: (event: ReactPointerEvent<T>) => void;
  onPointerCancel: (event: ReactPointerEvent<T>) => void;
  onClick: (event: ReactMouseEvent<T>) => void;
} {
  const activateRef = useRef(onActivate);
  activateRef.current = onActivate;
  const beforeTouchActivateRef = useRef(beforeTouchActivate);
  beforeTouchActivateRef.current = beforeTouchActivate;
  const startRef = useRef<TouchStart | null>(null);
  const suppressClickRef = useRef(false);

  const onPointerDown = useCallback((event: ReactPointerEvent<T>): void => {
    suppressClickRef.current = false;
    if (
      event.pointerType !== "touch" ||
      !event.isPrimary ||
      isNestedControl(event.target, event.currentTarget)
    ) {
      startRef.current = null;
      return;
    }
    startRef.current = {
      pointerId: event.pointerId,
      x: event.clientX,
      y: event.clientY,
      at: event.timeStamp,
    };
  }, []);

  const onPointerMove = useCallback((event: ReactPointerEvent<T>): void => {
    const start = startRef.current;
    if (!start || start.pointerId !== event.pointerId) return;
    if (
      Math.abs(event.clientX - start.x) > RELIABLE_TOUCH_TAP_MOVE_SLOP_PX ||
      Math.abs(event.clientY - start.y) > RELIABLE_TOUCH_TAP_MOVE_SLOP_PX
    ) {
      startRef.current = null;
    }
  }, []);

  const onPointerUp = useCallback((event: ReactPointerEvent<T>): void => {
    const start = startRef.current;
    startRef.current = null;
    if (
      !start ||
      start.pointerId !== event.pointerId ||
      event.timeStamp - start.at > MAX_TAP_MS
    ) return;
    suppressClickRef.current = true;
    beforeTouchActivateRef.current?.();
    activateRef.current();
  }, []);

  const onPointerCancel = useCallback((event: ReactPointerEvent<T>): void => {
    if (startRef.current?.pointerId === event.pointerId) startRef.current = null;
  }, []);

  const onClick = useCallback((event: ReactMouseEvent<T>): void => {
    const suppress = isPairedTouchClick(
      suppressClickRef.current,
      event.detail,
      "pointerType" in event.nativeEvent
        ? String(event.nativeEvent.pointerType)
        : "",
    );
    suppressClickRef.current = false;
    if (suppress) {
      event.preventDefault();
      return;
    }
    activateRef.current();
  }, []);

  return { onPointerDown, onPointerMove, onPointerUp, onPointerCancel, onClick };
}

/**
 * Consume the compatibility click from a completed touch before it can be
 * retargeted to a sibling action after layout changes. Attach the capture
 * handlers to the stable action-row owner and arm it immediately before the
 * touch activation that may reflow that row.
 */
export function useRetargetedTouchClickGuard<T extends HTMLElement>(): {
  arm: () => void;
  onPointerDownCapture: (event: ReactPointerEvent<T>) => void;
  onClickCapture: (event: ReactMouseEvent<T>) => void;
} {
  const guardRef = useRef<
    ReturnType<typeof createRetargetedTouchClickGuard> | null
  >(null);
  if (guardRef.current === null) {
    guardRef.current = createRetargetedTouchClickGuard();
  }

  const arm = useCallback((): void => {
    guardRef.current?.arm();
  }, []);
  const onPointerDownCapture = useCallback(
    (event: ReactPointerEvent<T>): void => {
      if (event.isPrimary) guardRef.current?.reset();
    },
    [],
  );
  const onClickCapture = useCallback((event: ReactMouseEvent<T>): void => {
    const suppress = guardRef.current?.consume(
      event.detail,
      "pointerType" in event.nativeEvent
        ? String(event.nativeEvent.pointerType)
        : "",
    ) ?? false;
    if (!suppress) return;
    event.preventDefault();
    event.stopPropagation();
  }, []);

  return { arm, onPointerDownCapture, onClickCapture };
}
