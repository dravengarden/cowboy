/** Desktop full screen for the Machine browser panel: the panel claims the
 * whole viewport (CSS) and, where the Fullscreen API exists, the window
 * leaves the desktop too. The document element is the fullscreen element so
 * menus and confirmations, which render in portals, stay visible. Esc exits
 * full screen before it may close the panel. */
import { useCallback, useEffect, useRef, useState } from "react";

/** An Esc that ended API full screen may still reach the dialog. */
const EXIT_GRACE_MS = 500;

function apiFullscreenActive(): boolean {
  return Boolean(globalThis.document?.fullscreenElement);
}

export interface BrowserFullscreen {
  fullscreen: boolean;
  toggle(): void;
  /** Wrap the panel's close: while (or just after) full screen, exit it instead. */
  guardClose(close: () => void): () => void;
}

export function useBrowserFullscreen(enabled: boolean): BrowserFullscreen {
  const [fullscreen, setFullscreen] = useState(false);
  const active = useRef(false);
  active.current = fullscreen;
  const ownsApi = useRef(false);
  const exitedAt = useRef(0);
  const exit = useCallback(() => {
    setFullscreen(false);
    if (ownsApi.current && apiFullscreenActive())
      void globalThis.document.exitFullscreen().catch(() => {});
    ownsApi.current = false;
  }, []);
  const enter = useCallback(() => {
    setFullscreen(true);
    const root = globalThis.document?.documentElement;
    if (!globalThis.document?.fullscreenEnabled || typeof root?.requestFullscreen !== "function")
      return;
    if (apiFullscreenActive()) return;
    ownsApi.current = true;
    root.requestFullscreen().catch(() => {
      // CSS full viewport remains the fallback.
      ownsApi.current = false;
    });
  }, []);
  useEffect(() => {
    const onChange = (): void => {
      if (!ownsApi.current || apiFullscreenActive()) return;
      // The window left full screen by itself (Esc): the same key must not
      // also close the panel.
      exitedAt.current = Date.now();
      exit();
    };
    globalThis.document?.addEventListener("fullscreenchange", onChange);
    return () => globalThis.document?.removeEventListener("fullscreenchange", onChange);
  }, [exit]);
  useEffect(() => {
    if (!enabled && active.current) exit();
  }, [enabled, exit]);
  useEffect(
    () => () => {
      if (ownsApi.current && apiFullscreenActive())
        void globalThis.document.exitFullscreen().catch(() => {});
    },
    [],
  );
  const toggle = useCallback(() => (active.current ? exit() : enter()), [enter, exit]);
  const guardClose = useCallback(
    (close: () => void) => () => {
      if (active.current || Date.now() - exitedAt.current < EXIT_GRACE_MS) exit();
      else close();
    },
    [exit],
  );
  return { fullscreen: enabled && fullscreen, toggle, guardClose };
}
