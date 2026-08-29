import { useCallback, useEffect, useRef, useState } from "react";
import { idleLockShouldEngage, noteActivity } from "./idleLock";

const ACTIVITY_EVENTS = ["pointerdown", "keydown", "touchstart", "scroll"] as const;

export function useIdlePasskeyLock(eligible: boolean, idleAfterMs: number): {
  locked: boolean;
  unlock: () => void;
} {
  const [locked, setLocked] = useState(false);
  const lastActiveMs = useRef(Date.now());
  const lockedRef = useRef(false);

  useEffect(() => {
    if (!eligible) {
      lockedRef.current = false;
      setLocked(false);
      lastActiveMs.current = Date.now();
    }
  }, [eligible]);

  useEffect(() => {
    let timer: number | undefined;
    const arm = (): void => {
      if (timer != null) globalThis.clearTimeout(timer);
      if (!eligible || lockedRef.current) return;
      const now = Date.now();
      if (
        idleLockShouldEngage({
          eligible,
          alreadyLocked: lockedRef.current,
          nowMs: now,
          lastActiveMs: lastActiveMs.current,
          idleAfterMs,
        })
      ) {
        lockedRef.current = true;
        setLocked(true);
        return;
      }
      const delay = Math.max(
        0,
        idleAfterMs - (now - lastActiveMs.current),
      );
      timer = globalThis.setTimeout(arm, delay);
    };
    const onActivity = (): void => {
      const next = noteActivity({
        alreadyLocked: lockedRef.current,
        nowMs: Date.now(),
      });
      if (next == null) return;
      lastActiveMs.current = next;
      arm();
    };
    for (const name of ACTIVITY_EVENTS) {
      globalThis.addEventListener(name, onActivity, { passive: true });
    }
    const onVisible = (): void => {
      if (document.visibilityState !== "visible") return;
      arm();
    };
    document.addEventListener("visibilitychange", onVisible);
    globalThis.addEventListener("focus", arm);
    arm();
    return () => {
      for (const name of ACTIVITY_EVENTS) {
        globalThis.removeEventListener(name, onActivity);
      }
      document.removeEventListener("visibilitychange", onVisible);
      globalThis.removeEventListener("focus", arm);
      if (timer != null) globalThis.clearTimeout(timer);
    };
  }, [eligible, idleAfterMs, locked]);

  const unlock = useCallback((): void => {
    lockedRef.current = false;
    lastActiveMs.current = Date.now();
    setLocked(false);
  }, []);

  return { locked, unlock };
}
