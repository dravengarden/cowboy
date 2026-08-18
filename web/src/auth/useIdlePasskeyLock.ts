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

  const markActive = useCallback((): void => {
    const next = noteActivity({ alreadyLocked: lockedRef.current, nowMs: Date.now() });
    if (next != null) lastActiveMs.current = next;
  }, []);

  const evaluate = useCallback((): void => {
    if (
      idleLockShouldEngage({
        eligible,
        alreadyLocked: lockedRef.current,
        nowMs: Date.now(),
        lastActiveMs: lastActiveMs.current,
        idleAfterMs,
      })
    ) {
      lockedRef.current = true;
      setLocked(true);
    }
  }, [eligible, idleAfterMs]);

  useEffect(() => {
    if (!eligible) {
      lockedRef.current = false;
      setLocked(false);
      lastActiveMs.current = Date.now();
    }
  }, [eligible]);

  useEffect(() => {
    const onActivity = (): void => {
      markActive();
    };
    for (const name of ACTIVITY_EVENTS) {
      globalThis.addEventListener(name, onActivity, { passive: true });
    }
    const onVisible = (): void => {
      if (document.visibilityState !== "visible") return;
      evaluate();
    };
    document.addEventListener("visibilitychange", onVisible);
    const timer = globalThis.setInterval(evaluate, 5_000);
    return () => {
      for (const name of ACTIVITY_EVENTS) {
        globalThis.removeEventListener(name, onActivity);
      }
      document.removeEventListener("visibilitychange", onVisible);
      globalThis.clearInterval(timer);
    };
  }, [evaluate, markActive]);

  const unlock = useCallback((): void => {
    lockedRef.current = false;
    lastActiveMs.current = Date.now();
    setLocked(false);
  }, []);

  return { locked, unlock };
}
