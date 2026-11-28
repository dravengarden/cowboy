import { isDesktopSplitterId } from "./desktopSplitterKeyboard";
import { activeDesktopComposerScope } from "./editorInsert";
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  paneChromeOwnsFocus,
  verticalWorkspaceRegion,
} from "./verticalWorkspaceRegion";
import {
  desktopPointerLeftComposer,
  desktopPointerLeftRegion,
  desktopRegionFromPointerTarget,
} from "./desktopComposerOwnership";
import {
  isDesktopRegionVisible,
  resolvePaneEntryRegion,
  resolveVisiblePointerRegion,
  resolveVisibleRegionElement,
} from "./desktopFocusResolution";
import {
  DESKTOP_COMPACT_WIDTH_QUERY,
  DESKTOP_PANES_EXPANDED,
  DESKTOP_SESSIONS_DRAWER_TOGGLE_EVENT,
  type DesktopCollapsedPanes,
  desktopCollapsedPanesStore,
  togglePaneCollapsed,
  toggleZoomPanes,
  useDesktopCollapsedPanes,
  withPaneCollapsed,
} from "../desktopLayout";

export type DesktopPane = "sessions" | "prompt" | "conversation";
export type WorkspaceMode = "normal" | "search" | "command";
export type DesktopSplitterId = "sessions-prompt" | "prompt-conversation" | "review-navigator";

interface DesktopWorkspaceContextValue {
  focusedPane: DesktopPane;
  focusedRegion: string | null;
  focusPane: (pane: DesktopPane) => void;
  focusRegion: (region: string) => void;
  focusAdjacentPane: (delta: -1 | 1) => void;
  focusAdjacentRegion: (delta: -1 | 1) => void;
  cycleRegion: () => void;
  mode: WorkspaceMode;
  setMode: (mode: WorkspaceMode) => void;
  selectedSplitter: DesktopSplitterId | null;
  setSelectedSplitter: (splitter: DesktopSplitterId | null) => void;
  collapsedPanes: DesktopCollapsedPanes;
  /** Collapse or restore a pane, keeping keyboard focus on a visible pane. */
  togglePane: (pane: DesktopPane) => void;
  /** Zoom the focused work pane to full width, or restore the prior layout. */
  toggleZoom: (sessionPage: boolean) => void;
  /** Restore Sessions, Prompt and Conversation and clear any zoom snapshot. */
  expandAllPanes: () => void;
}

const PANE_FALLBACK_REGION: Record<DesktopPane, string> = {
  sessions: "sessions.list",
  prompt: "prompt.composer",
  conversation: "conversation.transcript",
};

/** Run after React has committed a layout change and the browser laid it out. */
function afterLayout(action: () => void): void {
  requestAnimationFrame(() => requestAnimationFrame(action));
}

const DesktopWorkspaceContext = createContext<DesktopWorkspaceContextValue | null>(null);

function paneFromTarget(target: EventTarget | null): DesktopPane | null {
  if (!(target instanceof Element)) return null;
  const value = target.closest<HTMLElement>("[data-desktop-pane]")?.dataset.desktopPane;
  return value === "sessions" || value === "prompt" || value === "conversation"
    ? value
    : null;
}

/** Strictly maintain the single-focus invariant across visible Desktop regions. */
function markFocusedRegion(
  region: string | null,
  target?: EventTarget | null,
): HTMLElement | null {
  const preferred = target instanceof Element
    ? target.closest<HTMLElement>("[data-desktop-region]")
    : null;
  const targetEl =
    preferred &&
    preferred.dataset.desktopRegion === region &&
    isDesktopRegionVisible(preferred)
      ? preferred
      : region
      ? resolveVisibleRegionElement(region)
      : null;
  for (const element of document.querySelectorAll<HTMLElement>("[data-desktop-region]")) {
    if (element === targetEl) {
      if (element.dataset.desktopFocused !== "true") element.dataset.desktopFocused = "true";
    } else if (element.dataset.desktopFocused !== undefined) {
      delete element.dataset.desktopFocused;
    }
  }
  return targetEl;
}

function syncDesktopPaneChrome(
  focusedPane: DesktopPane,
  focusedRegion: string | null,
): void {
  const scope = document.querySelector("[data-desktop-workspace-root]") ?? document;
  for (const element of scope.querySelectorAll<HTMLElement>("[data-desktop-pane]")) {
    const pane = element.dataset.desktopPane;
    if (pane === "sessions" || pane === "prompt" || pane === "conversation") {
      if (paneChromeOwnsFocus(focusedPane, focusedRegion, pane)) {
        if (element.dataset.desktopPaneFocused !== "true") {
          element.dataset.desktopPaneFocused = "true";
        }
      } else if (element.dataset.desktopPaneFocused !== undefined) {
        delete element.dataset.desktopPaneFocused;
      }
    }
  }
}

function focusElement(element: HTMLElement | null): void {
  if (!element) return;
  const collapsedToggle = element.querySelector<HTMLElement>(
    "button[aria-label='Expand plan'], button[aria-label='expand']",
  );
  if (collapsedToggle) {
    collapsedToggle.click();
    requestAnimationFrame(() => focusElement(element));
    return;
  }
  const preferred = element.querySelector<HTMLElement>("[data-desktop-focus-default]");
  const currentItem = element.querySelector<HTMLElement>(
    "[data-desktop-item][data-desktop-current='true']",
  );
  const preferredItem = element.querySelector<HTMLElement>(
    "[data-desktop-item][data-desktop-preferred='true']",
  );
  const firstItem = element.querySelector<HTMLElement>("[data-desktop-item]");
  const composerScope = element.dataset.desktopRegion === "prompt.composer"
    ? activeDesktopComposerScope(element)
    : null;
  const composerCommandSink = composerScope
    ? composerScope.querySelector<HTMLElement>("[data-vim-command-sink]")
    : null;
  const composer = composerScope
    ? composerScope.querySelector<HTMLElement>(".cm-content[contenteditable='true']")
    : null;
  const planToggle = element.dataset.desktopRegion === "prompt.plan"
    ? element.querySelector<HTMLElement>(
      "button[aria-label='Expand plan'], button[aria-label='Collapse plan']",
    )
    : null;
  (composer ?? composerCommandSink ?? currentItem ?? preferredItem ?? preferred ?? firstItem ?? planToggle ?? element).focus({
    preventScroll: true,
  });
  element.scrollIntoView({ block: "nearest", inline: "nearest" });
}

function useDesktopPaneCollapseActions(
  focusedPaneRef: { readonly current: DesktopPane },
  resolveEntry: (pane: DesktopPane) => string,
  focusRegion: (region: string) => void,
) {
  const zoomRestoreRef = useRef<DesktopCollapsedPanes | null>(null);
  const togglePane = useCallback((pane: DesktopPane): void => {
    if (pane === "sessions" && globalThis.matchMedia?.(DESKTOP_COMPACT_WIDTH_QUERY).matches) {
      globalThis.dispatchEvent(new CustomEvent(DESKTOP_SESSIONS_DRAWER_TOGGLE_EVENT));
      return;
    }
    zoomRestoreRef.current = null;
    const current = desktopCollapsedPanesStore.get();
    const next = togglePaneCollapsed(current, pane);
    desktopCollapsedPanesStore.set(next);
    if (!next[pane]) {
      afterLayout(() => focusRegion(resolveEntry(pane)));
      return;
    }
    const active = document.activeElement;
    const paneElement = document.querySelector(`[data-desktop-pane="${pane}"]`);
    const focusInside = focusedPaneRef.current === pane ||
      (active instanceof Node && paneElement?.contains(active) === true);
    const swapped = (["prompt", "conversation"] as const).find((other) =>
      other !== pane && current[other] && !next[other]
    );
    if (!focusInside && !swapped) return;
    const target: DesktopPane = swapped ??
      (pane === "prompt" ? "conversation" : next.prompt ? "conversation" : "prompt");
    afterLayout(() => focusRegion(resolveEntry(target)));
  }, [focusRegion, focusedPaneRef, resolveEntry]);
  const toggleZoom = useCallback((sessionPage: boolean): void => {
    const compact = globalThis.matchMedia?.(DESKTOP_COMPACT_WIDTH_QUERY).matches ?? false;
    const result = toggleZoomPanes(
      desktopCollapsedPanesStore.get(),
      zoomRestoreRef.current,
      focusedPaneRef.current,
      sessionPage,
      compact,
    );
    zoomRestoreRef.current = result.saved;
    desktopCollapsedPanesStore.set(result.next);
    if (focusedPaneRef.current !== result.target) {
      afterLayout(() => focusRegion(resolveEntry(result.target)));
    }
  }, [focusRegion, focusedPaneRef, resolveEntry]);
  const expandAllPanes = useCallback((): void => {
    zoomRestoreRef.current = null;
    desktopCollapsedPanesStore.set(DESKTOP_PANES_EXPANDED);
  }, []);
  return { togglePane, toggleZoom, expandAllPanes };
}

export function DesktopWorkspaceProvider({
  children,
}: {
  children: React.ReactNode;
}): React.JSX.Element {
  const [focusedPane, setFocusedPane] = useState<DesktopPane>("prompt");
  const [focusedRegion, setFocusedRegion] = useState<string | null>("prompt.composer");
  const [mode, setMode] = useState<WorkspaceMode>("normal");
  const [selectedSplitter, setSelectedSplitter] = useState<DesktopSplitterId | null>(null);
  const collapsedPanes = useDesktopCollapsedPanes();
  const focusedPaneRef = useRef(focusedPane);
  focusedPaneRef.current = focusedPane;
  const focusedRegionRef = useRef(focusedRegion);
  focusedRegionRef.current = focusedRegion;
  const lastRegionByPaneRef = useRef<Partial<Record<DesktopPane, string>>>({});

  const resolveEntry = useCallback(
    (pane: DesktopPane): string =>
      resolvePaneEntryRegion(pane, lastRegionByPaneRef.current[pane]) ?? PANE_FALLBACK_REGION[pane],
    [],
  );

  const restoreCollapsedPane = useCallback((pane: DesktopPane | null): boolean => {
    if (!pane) return false;
    if (
      pane === "sessions" &&
      globalThis.matchMedia?.(DESKTOP_COMPACT_WIDTH_QUERY).matches
    ) return false;
    if (
      pane === "sessions" &&
      document.querySelector("[data-desktop-region='sessions.rail']")
    ) return false;
    const current = desktopCollapsedPanesStore.get();
    if (!current[pane]) return false;
    desktopCollapsedPanesStore.set(withPaneCollapsed(current, pane, false));
    return true;
  }, []);

  const focusRegion = useCallback((region: string): void => {
    const element = resolveVisibleRegionElement(region);
    if (!element) return;
    const pane = paneFromTarget(element);
    if (restoreCollapsedPane(pane)) {
      afterLayout(() => focusRegion(region));
      return;
    }
    if (pane) {
      setFocusedPane(pane);
      if (region !== "topbar.controls") lastRegionByPaneRef.current[pane] = region;
    }
    setFocusedRegion(region);
    markFocusedRegion(region, element);
    syncDesktopPaneChrome(pane ?? focusedPaneRef.current, region);
    focusElement(element);
  }, [restoreCollapsedPane]);

  const focusPane = useCallback((pane: DesktopPane): void => {
    if (restoreCollapsedPane(pane)) {
      afterLayout(() => focusPane(pane));
      return;
    }
    setFocusedPane(pane);
    const targetRegion = resolvePaneEntryRegion(pane, lastRegionByPaneRef.current[pane]);
    const regionEl = targetRegion ? resolveVisibleRegionElement(targetRegion) : null;
    if (targetRegion && regionEl) {
      lastRegionByPaneRef.current[pane] = targetRegion;
      setFocusedRegion(targetRegion);
      markFocusedRegion(targetRegion, regionEl);
      syncDesktopPaneChrome(pane, targetRegion);
      focusElement(regionEl);
    } else {
      const paneElement = document.querySelector<HTMLElement>(`[data-desktop-pane="${pane}"]`);
      syncDesktopPaneChrome(pane, null);
      focusElement(paneElement ?? null);
    }
  }, [restoreCollapsedPane]);

  const focusAdjacentPane = useCallback((delta: -1 | 1): void => {
    const order: DesktopPane[] = ["sessions", "prompt", "conversation"];
    const available = order.filter((pane) =>
      !collapsedPanes[pane] &&
      document.querySelector(`[data-desktop-pane="${pane}"]`)
    );
    if (available.length === 0) return;
    const current = Math.max(0, available.indexOf(focusedPane));
    focusPane(available[(current + delta + available.length) % available.length] as DesktopPane);
  }, [collapsedPanes, focusPane, focusedPane]);

  const { togglePane, toggleZoom, expandAllPanes } = useDesktopPaneCollapseActions(
    focusedPaneRef,
    resolveEntry,
    focusRegion,
  );

  const focusAdjacentRegion = useCallback((delta: -1 | 1): void => {
    const next =
      focusedRegion === "topbar.controls" && delta === 1
        ? resolveEntry(focusedPane)
        : verticalWorkspaceRegion(focusedPane, focusedRegion, delta);
    if (next && resolveVisibleRegionElement(next)) {
      focusRegion(next);
    }
  }, [focusRegion, focusedPane, focusedRegion, resolveEntry]);

  const cycleRegion = useCallback((): void => {
    const regions = [...document.querySelectorAll<HTMLElement>("[data-desktop-region]")]
      .filter((element) => isDesktopRegionVisible(element));
    if (regions.length === 0) return;
    const current = Math.max(
      0,
      regions.findIndex((element) => element.dataset.desktopRegion === focusedRegion),
    );
    const next = regions[(current + 1) % regions.length];
    if (next?.dataset.desktopRegion) focusRegion(next.dataset.desktopRegion);
  }, [focusRegion, focusedRegion]);

  useEffect(() => {
    const syncPane = (event: Event): void => {
      if (event.type === "pointerdown" && event.target instanceof Element) {
        const splitter = event.target.closest<HTMLElement>("[data-desktop-splitter]")
          ?.dataset.desktopSplitter;
        setSelectedSplitter(isDesktopSplitterId(splitter) ? splitter : null);
      }
      const pane = paneFromTarget(event.target);
      if (pane) setFocusedPane(pane);
      const region = resolveVisiblePointerRegion(event.target, lastRegionByPaneRef.current) ?? desktopRegionFromPointerTarget(event.target);
      if (
        event.type === "pointerdown" &&
        (desktopPointerLeftComposer(event.target, document.activeElement) ||
          desktopPointerLeftRegion(event.target, document.activeElement))
      ) {
        const clickedFocusable = event.target instanceof Element
          ? event.target.closest<HTMLElement>(
            "button, a, input, textarea, select, [href], [contenteditable='true']",
          )
          : null;
        if (!clickedFocusable && region) {
          const regionElement = resolveVisibleRegionElement(region);
          if (regionElement) focusElement(regionElement);
        } else if (document.activeElement instanceof HTMLElement) {
          document.activeElement.blur();
        }
      }
      if (region) {
        if (pane && region !== "topbar.controls") lastRegionByPaneRef.current[pane] = region;
        setFocusedRegion(region);
        markFocusedRegion(region, event.target);
      }
      if (pane || region === "topbar.controls") {
        const nextPane = pane ?? focusedPaneRef.current;
        const nextRegion = region === "topbar.controls" ? "topbar.controls" : region;
        if (
          nextPane !== focusedPaneRef.current ||
          nextRegion === "topbar.controls" ||
          focusedRegionRef.current === "topbar.controls"
        ) {
          syncDesktopPaneChrome(nextPane, nextRegion);
        }
      }
    };
    document.addEventListener("pointerdown", syncPane, true);
    document.addEventListener("focusin", syncPane, true);
    return () => {
      document.removeEventListener("pointerdown", syncPane, true);
      document.removeEventListener("focusin", syncPane, true);
    };
  }, []);

  useEffect(() => {
    if (selectedSplitter === null) return undefined;
    const frame = requestAnimationFrame(() => {
      const splitter = document.querySelector<HTMLElement>(
        `[data-desktop-splitter="${CSS.escape(selectedSplitter)}"]`,
      );
      if (!splitter || splitter.offsetParent === null) setSelectedSplitter(null);
    });
    return (): void => cancelAnimationFrame(frame);
  }, [collapsedPanes, selectedSplitter]);

  useEffect(() => {
    const syncMountedWorkspace = (): boolean => {
      syncDesktopPaneChrome(focusedPane, focusedRegion);
      const focusedElement = markFocusedRegion(focusedRegion);
      if (!focusedElement) return false;
      if (
        focusedElement.dataset.desktopRegion === "prompt.composer" &&
        !focusedElement.querySelector(
          "[data-vim-command-sink], .cm-content[contenteditable='true']",
        )
      ) {
        return false;
      }
      if (document.activeElement === document.body) focusElement(focusedElement);
      return true;
    };

    if (syncMountedWorkspace()) return undefined;
    const root = document.getElementById("root") ?? document.body;
    const observer = new MutationObserver(() => {
      if (syncMountedWorkspace()) observer.disconnect();
    });
    observer.observe(root, { childList: true, subtree: true });
    return (): void => observer.disconnect();
  }, [focusedPane, focusedRegion]);

  const value = useMemo<DesktopWorkspaceContextValue>(() => ({
    focusedPane,
    focusedRegion,
    focusPane,
    focusRegion,
    focusAdjacentPane,
    focusAdjacentRegion,
    cycleRegion,
    mode,
    setMode,
    selectedSplitter,
    setSelectedSplitter,
    collapsedPanes,
    togglePane,
    toggleZoom,
    expandAllPanes,
  }), [
    collapsedPanes,
    togglePane,
    toggleZoom,
    expandAllPanes,
    cycleRegion,
    focusAdjacentPane,
    focusAdjacentRegion,
    focusPane,
    focusRegion,
    focusedPane,
    focusedRegion,
    mode,
    selectedSplitter,
  ]);

  return (
    <DesktopWorkspaceContext.Provider value={value}>
      {children}
    </DesktopWorkspaceContext.Provider>
  );
}

export function useDesktopWorkspace(): DesktopWorkspaceContextValue {
  const value = useContext(DesktopWorkspaceContext);
  if (!value) throw new Error("useDesktopWorkspace must be used inside DesktopWorkspaceProvider");
  return value;
}

export function useOptionalDesktopWorkspace(): DesktopWorkspaceContextValue | null {
  return useContext(DesktopWorkspaceContext);
}
