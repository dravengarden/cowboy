import { useSyncExternalStore } from "react";
import { clearTranscriptViewport } from "../transcriptViewportStore";
import {
  type ExploreSessionState,
  exploreStateAfterContextClear,
  type TranscriptProjection,
} from "./contextClear.ts";

export type { ExploreSessionState, TranscriptProjection } from "./contextClear.ts";

const STORAGE_KEY = "cowboy:transcript-projections:v1";
const states = new Map<string, ExploreSessionState>();
const tailStates = new Map<string, boolean>();
const listeners = new Set<() => void>();
const DEFAULT_STATE: ExploreSessionState = {
  projection: "history",
  pageId: null,
  pageStartId: null,
  pageLoadingId: null,
  transitionAnchorKey: null,
  followTailRequested: false,
  pageParents: {},
  pendingFollowUp: null,
};

function restore(): void {
  if (typeof localStorage === "undefined") return;
  try {
    const raw = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "{}") as Record<
      string,
      Partial<ExploreSessionState>
    >;
    for (const [sessionId, state] of Object.entries(raw)) {
      const pageId = typeof state.pageId === "string" ? state.pageId : null;
      const projection = state.projection === "explore" ? "explore" : "history";
      states.set(sessionId, {
        projection,
        pageId,
        // A restored Page View is a continuation of the device-local reading
        // position. Only explicit page navigation requests pageStartId; setting
        // it here raced Transcript's viewport restore back to the page head.
        pageStartId: null,
        pageLoadingId: null,
        transitionAnchorKey: null,
        followTailRequested: false,
        pageParents: state.pageParents && typeof state.pageParents === "object"
          ? state.pageParents
          : {},
        pendingFollowUp: state.pendingFollowUp &&
            typeof state.pendingFollowUp === "object"
          ? state.pendingFollowUp
          : null,
      });
    }
  } catch {
    // A stale preference must never prevent Cowboy from rendering History.
  }
}

restore();

function persist(): void {
  if (typeof localStorage === "undefined") return;
  localStorage.setItem(
    STORAGE_KEY,
    JSON.stringify(Object.fromEntries(
      [...states.entries()].map(([sessionId, state]) => [
        sessionId,
        {
          projection: state.projection,
          pageId: state.pageId,
          pageParents: state.pageParents,
          pendingFollowUp: state.pendingFollowUp,
        },
      ]),
    )),
  );
}

function emit(): void {
  for (const listener of listeners) listener();
}

function get(sessionId: string): ExploreSessionState {
  return states.get(sessionId) ?? DEFAULT_STATE;
}

function update(
  sessionId: string,
  patch: Partial<ExploreSessionState>,
): void {
  const current = get(sessionId);
  const next = { ...current, ...patch };
  if (
    current.projection === next.projection &&
    current.pageId === next.pageId &&
    current.pageStartId === next.pageStartId &&
    current.pageLoadingId === next.pageLoadingId &&
    current.transitionAnchorKey === next.transitionAnchorKey &&
    current.followTailRequested === next.followTailRequested &&
    current.pageParents === next.pageParents &&
    current.pendingFollowUp === next.pendingFollowUp
  ) return;
  states.set(sessionId, next);
  persist();
  emit();
}

export function setTranscriptProjection(
  sessionId: string,
  projection: TranscriptProjection,
  transitionAnchorKey: string | null = null,
): void {
  if (projection === "explore" && get(sessionId).projection !== "explore") {
    clearTranscriptViewport(sessionId, "page");
  }
  update(sessionId, {
    projection,
    transitionAnchorKey,
    followTailRequested: false,
  });
}

export function setExplorePage(sessionId: string, pageId: string | null): void {
  update(sessionId, { pageId, followTailRequested: false });
}

export function resetExploreAfterContextClear(sessionId: string): void {
  clearTranscriptViewport(sessionId, "page");
  tailStates.delete(sessionId);
  states.set(sessionId, exploreStateAfterContextClear(get(sessionId)));
  persist();
  emit();
}

export function setExploreAtTail(sessionId: string, atTail: boolean): void {
  if ((tailStates.get(sessionId) ?? false) === atTail) return;
  if (atTail) tailStates.set(sessionId, true);
  else tailStates.delete(sessionId);
  emit();
}

export function navigateExplorePage(sessionId: string, pageId: string): void {
  clearTranscriptViewport(sessionId, "page");
  update(sessionId, {
    pageId,
    pageStartId: pageId,
    pageLoadingId: pageId,
    followTailRequested: false,
  });
}

/** Leave an older retained page and let Page View resolve its live tail again. */
export function followExploreTail(sessionId: string): void {
  clearTranscriptViewport(sessionId, "page");
  update(sessionId, {
    pageId: null,
    pageStartId: null,
    pageLoadingId: null,
    followTailRequested: true,
  });
}

export function resolveExploreTail(sessionId: string, pageId: string): void {
  update(sessionId, {
    pageId,
    pageStartId: null,
    pageLoadingId: null,
    followTailRequested: false,
  });
}

export function beginExplorePageLoading(sessionId: string): void {
  update(sessionId, { pageLoadingId: "" });
}

export function resolveExplorePageStart(sessionId: string): void {
  update(sessionId, { pageStartId: null, pageLoadingId: null });
}

export function resolveProjectionAnchor(
  sessionId: string,
  pageId?: string | null,
): void {
  update(sessionId, {
    ...(pageId === undefined ? {} : { pageId }),
    transitionAnchorKey: null,
  });
}

export function captureTranscriptViewportAnchor(
  sessionId: string,
): string | null {
  if (typeof document === "undefined") return null;
  const scroller = document.querySelector<HTMLElement>(
    `[data-transcript-session="${CSS.escape(sessionId)}"]`,
  );
  if (!scroller) return null;
  const viewport = scroller.getBoundingClientRect();
  const centre = viewport.top + viewport.height / 2;
  let nearest: { key: string; distance: number } | null = null;
  for (const row of scroller.querySelectorAll<HTMLElement>("[data-key]")) {
    const key = row.dataset["key"];
    if (!key) continue;
    const rect = row.getBoundingClientRect();
    if (rect.bottom < viewport.top || rect.top > viewport.bottom) continue;
    const distance = centre < rect.top
      ? rect.top - centre
      : centre > rect.bottom
      ? centre - rect.bottom
      : 0;
    if (!nearest || distance < nearest.distance) nearest = { key, distance };
    if (distance === 0) break;
  }
  return nearest?.key ?? null;
}

export function queueExploreFollowUp(
  sessionId: string,
  targetPageId: string,
  knownPageIds: string[],
): void {
  update(sessionId, {
    pendingFollowUp: { targetPageId, knownPageIds },
  });
}

export function resolveExploreFollowUp(
  sessionId: string,
  pageIds: string[],
): void {
  const state = get(sessionId);
  const pending = state.pendingFollowUp;
  if (!pending) return;
  const nextPageId = pageIds.find((id) => !pending.knownPageIds.includes(id));
  if (!nextPageId) return;
  update(sessionId, {
    pageId: pending.targetPageId,
    pageParents: {
      ...state.pageParents,
      [nextPageId]: pending.targetPageId,
    },
    pendingFollowUp: null,
  });
}

export function useExploreSessionState(
  sessionId: string,
): ExploreSessionState {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    () => get(sessionId),
    () => DEFAULT_STATE,
  );
}

export function useExploreAtTail(sessionId: string): boolean {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    () => tailStates.get(sessionId) ?? false,
    () => false,
  );
}
