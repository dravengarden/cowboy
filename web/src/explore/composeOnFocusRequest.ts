import { useEffect, useRef } from "react";
import { COMPOSER_FOCUS_EVENT } from "../sessionSettingsOpen";
import { useReliableTouchTap } from "../useReliableTouchTap";

/** Page View mounts its Composer only on a compose intent. A request to focus
 *  this session's Composer (a message put back in the Prompt, a loaded
 *  pending message) opens it, so the restored draft is on screen; an open
 *  Composer handles the request itself. */
export function useComposeOnFocusRequest(
  sessionId: string,
  composeOpen: boolean,
  open: () => void,
): void {
  const openRef = useRef(open);
  openRef.current = open;
  useEffect(() => {
    if (composeOpen) return undefined;
    const listener = (event: Event): void => {
      const target = event instanceof CustomEvent
        ? (event.detail as { sessionId?: unknown } | null)?.sessionId
        : undefined;
      if (target === sessionId) openRef.current();
    };
    document.addEventListener(COMPOSER_FOCUS_EVENT, listener);
    return () => document.removeEventListener(COMPOSER_FOCUS_EVENT, listener);
  }, [composeOpen, sessionId]);
}

/** The Page Dock's compose toggle: its reliable touch tap, and the same
 *  intent opened by a focus request while the Composer is unmounted. */
export function useComposeToggle(
  sessionId: string,
  composeOpen: boolean,
  toggle: () => void,
): ReturnType<typeof useReliableTouchTap<HTMLButtonElement>> {
  useComposeOnFocusRequest(sessionId, composeOpen, toggle);
  return useReliableTouchTap<HTMLButtonElement>(toggle);
}
