import { useCallback, useEffect, useRef } from "react";
import type {
  MouseEvent as ReactMouseEvent,
  PointerEvent as ReactPointerEvent,
} from "react";
import { RELIABLE_TOUCH_TAP_MOVE_SLOP_PX } from "./touchGestures";

const MAX_TAP_MS = 800;

interface TouchStart {
  pointerId: number;
  x: number;
  y: number;
  at: number;
}

function isNestedControl(target: EventTarget | null, current: HTMLElement): boolean {
  if (!(target instanceof Element)) return false;
  const control = target.closest("button, a, input, select, textarea, [role='button']");
  return control !== null && control !== current;
}

/**
 * Preserve normal click/keyboard activation while committing a stationary touch
 * on pointerup. Mobile Safari can omit the later synthetic click when the touch
 * first stops momentum in a scroll container; pointerup is still delivered.
 * A movement threshold keeps scrolling and drag gestures entirely native.
 */
export function useReliableTouchTap<T extends HTMLElement>(onActivate: () => void): {
  onPointerDown: (event: ReactPointerEvent<T>) => void;
  onPointerMove: (event: ReactPointerEvent<T>) => void;
  onPointerUp: (event: ReactPointerEvent<T>) => void;
  onPointerCancel: (event: ReactPointerEvent<T>) => void;
  onClick: (event: ReactMouseEvent<T>) => void;
} {
  const activateRef = useRef(onActivate);
  activateRef.current = onActivate;
  const startRef = useRef<TouchStart | null>(null);
  const suppressClickRef = useRef(false);
  const clearTimerRef = useRef<number | null>(null);

  const clearSuppressionLater = useCallback((): void => {
    if (clearTimerRef.current !== null) globalThis.clearTimeout(clearTimerRef.current);
    clearTimerRef.current = globalThis.setTimeout(() => {
      suppressClickRef.current = false;
      clearTimerRef.current = null;
    }, 700);
  }, []);

  useEffect(() => (): void => {
    if (clearTimerRef.current !== null) globalThis.clearTimeout(clearTimerRef.current);
  }, []);

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
    clearSuppressionLater();
    activateRef.current();
  }, [clearSuppressionLater]);

  const onPointerCancel = useCallback((event: ReactPointerEvent<T>): void => {
    if (startRef.current?.pointerId === event.pointerId) startRef.current = null;
  }, []);

  const onClick = useCallback((event: ReactMouseEvent<T>): void => {
    if (suppressClickRef.current) {
      suppressClickRef.current = false;
      event.preventDefault();
      return;
    }
    activateRef.current();
  }, []);

  return { onPointerDown, onPointerMove, onPointerUp, onPointerCancel, onClick };
}
