import { useCallback, useEffect, useRef } from "react";
import type {
  MouseEvent as ReactMouseEvent,
  PointerEvent as ReactPointerEvent,
} from "react";
import {
  type BackdropClickGuard,
  shouldBlockBackdropClick,
} from "./backdropDismiss";

const MAX_TAP_MS = 800;
const MOVE_SLOP_PX = 12;
const COMPAT_CLICK_GUARD_MS = 700;

interface BackdropPress {
  pointerId: number;
  x: number;
  y: number;
  at: number;
}

/**
 * Dismiss an overlay on pointerup without allowing WebKit's following
 * compatibility click to hit the newly exposed page. The document capture
 * guard survives the backdrop's unmount and consumes only the paired click at
 * the same screen coordinate; keyboard and later unrelated clicks remain
 * native.
 */
export function useBackdropDismiss<T extends HTMLElement>(
  onDismiss: () => void,
): {
  onPointerDown: (event: ReactPointerEvent<T>) => void;
  onPointerMove: (event: ReactPointerEvent<T>) => void;
  onPointerUp: (event: ReactPointerEvent<T>) => void;
  onPointerCancel: (event: ReactPointerEvent<T>) => void;
  onClick: (event: ReactMouseEvent<T>) => void;
} {
  const dismissRef = useRef(onDismiss);
  dismissRef.current = onDismiss;
  const pressRef = useRef<BackdropPress | null>(null);
  const cleanupGuardRef = useRef<(() => void) | null>(null);

  const clearGuard = useCallback((): void => {
    cleanupGuardRef.current?.();
    cleanupGuardRef.current = null;
  }, []);

  useEffect(() => clearGuard, [clearGuard]);

  const armClickGuard = useCallback((x: number, y: number): void => {
    clearGuard();
    const document = globalThis.document;
    if (!document) return;
    const guard: BackdropClickGuard = {
      x,
      y,
      expiresAt: Date.now() + COMPAT_CLICK_GUARD_MS,
    };
    let timer = 0;
    const cleanup = (): void => {
      document.removeEventListener("click", consume, true);
      globalThis.clearTimeout(timer);
      if (cleanupGuardRef.current === cleanup) cleanupGuardRef.current = null;
    };
    const consume = (event: MouseEvent): void => {
      if (!shouldBlockBackdropClick(guard, event, Date.now())) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      cleanup();
    };
    document.addEventListener("click", consume, true);
    timer = globalThis.setTimeout(cleanup, COMPAT_CLICK_GUARD_MS);
    cleanupGuardRef.current = cleanup;
  }, [clearGuard]);

  const onPointerDown = useCallback((event: ReactPointerEvent<T>): void => {
    event.stopPropagation();
    if (!event.isPrimary || event.button !== 0) {
      pressRef.current = null;
      return;
    }
    pressRef.current = {
      pointerId: event.pointerId,
      x: event.clientX,
      y: event.clientY,
      at: event.timeStamp,
    };
    event.currentTarget.setPointerCapture(event.pointerId);
  }, []);

  const onPointerMove = useCallback((event: ReactPointerEvent<T>): void => {
    event.stopPropagation();
    const press = pressRef.current;
    if (!press || press.pointerId !== event.pointerId) return;
    if (
      Math.abs(event.clientX - press.x) > MOVE_SLOP_PX ||
      Math.abs(event.clientY - press.y) > MOVE_SLOP_PX
    ) {
      pressRef.current = null;
    }
  }, []);

  const onPointerUp = useCallback((event: ReactPointerEvent<T>): void => {
    event.preventDefault();
    event.stopPropagation();
    const press = pressRef.current;
    pressRef.current = null;
    if (
      !press || press.pointerId !== event.pointerId ||
      event.timeStamp - press.at > MAX_TAP_MS
    ) return;
    armClickGuard(event.clientX, event.clientY);
    dismissRef.current();
  }, [armClickGuard]);

  const onPointerCancel = useCallback((event: ReactPointerEvent<T>): void => {
    event.preventDefault();
    event.stopPropagation();
    if (pressRef.current?.pointerId === event.pointerId) {
      pressRef.current = null;
    }
  }, []);

  const onClick = useCallback((event: ReactMouseEvent<T>): void => {
    event.preventDefault();
    event.stopPropagation();
    dismissRef.current();
  }, []);

  return {
    onPointerDown,
    onPointerMove,
    onPointerUp,
    onPointerCancel,
    onClick,
  };
}
