import { useEffect, useMemo, useRef, useState } from "react";
import { newUuid } from "../../uuid";
import {
  closeCodeBuffer,
  type CodeDiagnostic,
  type CodeHover,
  type CodeLocation,
  type CodeNavigationKind,
  fetchCodeHover,
  fetchCodeLanguage,
  fetchCodeNavigation,
  openCodeBuffer,
} from "../../mobile/review/codeApi";
import { useOwnedReviewBuffer } from "../../mobile/review/useOwnedReviewBuffer";

export type ReviewLanguageState = "off" | "starting" | "ready" | "unavailable";

export interface ReviewLanguage {
  readonly state: ReviewLanguageState;
  readonly diagnostics: readonly CodeDiagnostic[];
  /** Points are zero-based rows and columns of the source file. */
  readonly hover: (row: number, column: number, signal: AbortSignal) => Promise<CodeHover>;
  /** Locations, or null when this mode cannot navigate (owned buffers, as on the phone). */
  readonly navigate: (row: number, column: number, kind: CodeNavigationKind, signal: AbortSignal) => Promise<readonly CodeLocation[] | null>;
}

/**
 * Language intelligence for an open source file, in whichever mode the
 * context's manifest names. Owned buffers check every answer against the
 * file text Review shows (useOwnedReviewBuffer); legacy holds a buffer
 * lease while the file is open and reads the language routes. Navigation
 * exists only in legacy mode, as on the phone.
 */
export function useReviewLanguage(
  sessionId: string,
  path: string | undefined,
  mode: "legacy" | "owned" | "unavailable" | undefined,
  text: string | undefined,
  dataRevision: number,
): ReviewLanguage {
  const owned = useOwnedReviewBuffer(sessionId, path ?? "", mode === "owned" && path !== undefined, mode === "owned" ? text : undefined);
  const [legacy, setLegacy] = useState<{ key: string; state: ReviewLanguageState; diagnostics: CodeDiagnostic[] }>();
  const leased = useRef<string | undefined>(undefined);
  const key = path && mode === "legacy" ? `${sessionId}\0${path}\0${String(dataRevision)}` : undefined;
  useEffect(() => {
    if (!key || !path) return undefined;
    const lease = newUuid();
    let open = false;
    let released = false;
    const timers: number[] = [];
    setLegacy({ key, state: "starting", diagnostics: [] });
    const read = (): void => {
      void fetchCodeLanguage(sessionId, path).then((language) => {
        if (!released) setLegacy({ key, state: "ready", diagnostics: language.diagnostics });
      }).catch(() => undefined);
    };
    void openCodeBuffer(sessionId, path, lease).then(() => {
      open = true;
      leased.current = key;
      if (released) {
        void closeCodeBuffer(sessionId, path, lease).catch(() => undefined);
        return;
      }
      read();
      // A language server starts on the first buffer; read again after the
      // usual warm and cold start windows (as the phone does).
      timers.push(globalThis.setTimeout(read, 8_000), globalThis.setTimeout(read, 30_000));
    }).catch(() => {
      if (!released) setLegacy({ key, state: "unavailable", diagnostics: [] });
    });
    return () => {
      released = true;
      timers.forEach((timer) => globalThis.clearTimeout(timer));
      if (leased.current === key) leased.current = undefined;
      if (open) void closeCodeBuffer(sessionId, path, lease).catch(() => undefined);
    };
  }, [key, path, sessionId]);
  return useMemo((): ReviewLanguage => {
    if (mode === "owned" && path) {
      const state: ReviewLanguageState = owned.status === "ready"
        ? "ready"
        : owned.status === "unavailable" || owned.status === "mismatch"
        ? "unavailable"
        : "starting";
      return {
        state,
        diagnostics: owned.language?.diagnostics ?? [],
        hover: (row, column, signal) => owned.hover(row, column, signal),
        navigate: () => Promise.resolve(null),
      };
    }
    if (mode === "legacy" && path) {
      const current = legacy?.key === key ? legacy : undefined;
      return {
        state: current?.state ?? "starting",
        diagnostics: current?.diagnostics ?? [],
        hover: (row, column, signal) => fetchCodeHover(sessionId, path, row, column, signal),
        navigate: async (row, column, kind, signal) =>
          (await fetchCodeNavigation(sessionId, path, row, column, kind, signal)).locations,
      };
    }
    return {
      state: mode === undefined ? "starting" : path ? "unavailable" : "off",
      diagnostics: [],
      hover: () => Promise.reject(new Error("unavailable")),
      navigate: () => Promise.resolve(null),
    };
  }, [key, legacy, mode, owned, path, sessionId]);
}
