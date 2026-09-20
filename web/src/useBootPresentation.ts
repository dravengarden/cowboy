import { useEffect, useRef } from "react";
import { installBootSnapshot, signalBootReady, type BootSnapshotContext } from "./bootSnapshot";
import { productSyncPrincipal } from "./productSyncIdentity";

/** The app side of the boot presentation (docs/offline-first-sync.md §Boot
 * presentation): tell the overlay when the real screen has arrived, and save
 * that screen for the next open. */
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
  useEffect(() =>
    installBootSnapshot((): BootSnapshotContext | null => {
      const current = contextRef.current;
      const user = productSyncPrincipal();
      // No bound principal, or nothing painted yet: there is no screen of
      // this user's to save.
      if (user === undefined || !current.painted) return null;
      return {
        user,
        sessionId: current.sessionId,
        themeMode: current.themeMode,
        busy: current.busy,
      };
    }), []);
}
