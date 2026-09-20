import { useEffect, useRef } from "react";
import {
  type BootSnapshotContext,
  captureBootSnapshot,
  installBootSnapshot,
  signalBootReady,
} from "./bootSnapshot";
import { productSyncPrincipal } from "./productSyncIdentity";

/** How long the screen must hold still before it is worth saving. Long enough
 * that opening a session, scrolling and a finishing turn do not each cost a
 * capture; short enough that leaving soon after reading still saved it. */
const SETTLE_MS = 2_500;

/** The app side of the boot presentation (docs/offline-first-sync.md §Boot
 * presentation): tell the overlay when the real screen has arrived, and keep
 * a copy of that screen for the next open. */
export function useBootPresentation(input: {
  /** The active session's content is on screen, or there is none to show. */
  readonly painted: boolean;
  readonly sessionId: string | null;
  readonly themeMode: string;
  readonly busy: boolean;
}): void {
  useEffect(() => {
    if (input.painted) signalBootReady();
  }, [input.painted]);

  // The capture reads the live DOM when it fires, so the listeners are
  // installed once and only the context they read is kept current.
  const contextRef = useRef(input);
  contextRef.current = input;
  const read = useRef((): BootSnapshotContext | null => {
    const current = contextRef.current;
    const user = productSyncPrincipal();
    // No bound principal, or nothing painted yet: there is no screen of this
    // user's to save.
    if (user === undefined || !current.painted) return null;
    return {
      user,
      sessionId: current.sessionId,
      themeMode: current.themeMode,
      busy: current.busy,
    };
  });
  useEffect(() => installBootSnapshot(read.current), []);

  // The capture the next open actually depends on. A document being discarded
  // cannot finish an asynchronous Cache Storage write, so saving the screen as
  // the user leaves is a bonus rather than the mechanism; what gets restored
  // is whatever was saved a few seconds after the screen last settled.
  // Re-armed whenever what is on screen changes.
  useEffect(() => {
    if (!input.painted || input.busy) return undefined;
    const timer = setTimeout(() => {
      const current = read.current();
      if (current !== null) void captureBootSnapshot(current);
    }, SETTLE_MS);
    return () => clearTimeout(timer);
  }, [input.painted, input.busy, input.sessionId]);
}
