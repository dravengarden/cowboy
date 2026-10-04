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
  DESKTOP_COMPACT_WIDTH_QUERY,
  DESKTOP_SESSIONS_DRAWER_TOGGLE_EVENT,
  type DesktopCollapsedPanes,
  desktopCollapsedPanesStore,
  togglePaneCollapsed,
  useDesktopCollapsedPanes,
  withPaneCollapsed,
} from "../desktopLayout";

export type DesktopPane = "sessions" | "prompt" | "conversation";
export type WorkspaceMode = "normal" | "search" | "command";
export type DesktopProductMode = "agent" | "reading" | "code";
export type DesktopSplitterId =
  | "sessions-prompt"
  | "prompt-conversation"
  | "questions-page";

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
  productMode: DesktopProductMode;
  setProductMode: (mode: DesktopProductMode) => void;
  readingSidebarOpen: boolean;
  setReadingSidebarOpen: (open: boolean) => void;
  selectedSplitter: DesktopSplitterId | null;
  setSelectedSplitter: (splitter: DesktopSplitterId | null) => void;
  collapsedPanes: DesktopCollapsedPanes;
  /** Collapse or restore a pane, keeping keyboard focus on a visible pane. */
  togglePane: (pane: DesktopPane) => void;
}

/** Where keyboard focus lands when a pane is restored by its own command. */
const PANE_ENTRY_REGION: Record<DesktopPane, string> = {
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

function regionFromTarget(target: EventTarget | null): string | null {
  if (!(target instanceof Element)) return null;
  return target.closest<HTMLElement>("[data-desktop-region]")?.dataset.desktopRegion ?? null;
}

function syncDesktopPaneChrome(
  focusedPane: DesktopPane,
  focusedRegion: string | null,
): void {
  for (const element of document.querySelectorAll<HTMLElement>("[data-desktop-pane]")) {
    const pane = element.dataset.desktopPane;
    if (
      pane === "sessions" || pane === "prompt" || pane === "conversation"
    ) {
      if (paneChromeOwnsFocus(focusedPane, focusedRegion, pane)) {
        element.dataset.desktopPaneFocused = "true";
      } else {
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
  const firstItem = element.querySelector<HTMLElement>("[data-desktop-item]");
  const composerCommandSink = element.dataset.desktopRegion === "prompt.composer"
    ? element.querySelector<HTMLElement>("[data-vim-command-sink]")
    : null;
  const composer = element.dataset.desktopRegion === "prompt.composer"
    ? element.querySelector<HTMLElement>(".cm-content[contenteditable='true']")
    : null;
  const planToggle = element.dataset.desktopRegion === "prompt.plan"
    ? element.querySelector<HTMLElement>(
      "button[aria-label='Expand plan'], button[aria-label='Collapse plan']",
    )
    : null;
  // Entering Sessions must anchor navigation on the session already open in
  // the workspace. Falling back to row one made the first J/K jump unrelated
  // to what the user was looking at. Other list regions retain their first-row
  // default, while explicit focus targets still beat that generic fallback.
  // Focusing Prompt must not change the editor's modal state or caret. Focus
  // CodeMirror's content surface first; the Vim runtime transfers focus to its
  // command sink itself when the preserved state is Normal/Visual, while Insert
  // retains the native editable and its selection.
  (composer ?? composerCommandSink ?? currentItem ?? preferred ?? firstItem ?? planToggle ?? element).focus({
    preventScroll: true,
  });
  element.scrollIntoView({ block: "nearest", inline: "nearest" });
}

export function DesktopWorkspaceProvider({
  children,
}: {
  children: React.ReactNode;
}): React.JSX.Element {
  const [focusedPane, setFocusedPane] = useState<DesktopPane>("prompt");
  const [focusedRegion, setFocusedRegion] = useState<string | null>("prompt.composer");
  const [mode, setMode] = useState<WorkspaceMode>("normal");
  const [productMode, setProductMode] = useState<DesktopProductMode>("agent");
  const [readingSidebarOpen, setReadingSidebarOpen] = useState(false);
  const [selectedSplitter, setSelectedSplitter] = useState<DesktopSplitterId | null>(null);
  const collapsedPanes = useDesktopCollapsedPanes();
  const focusedPaneRef = useRef(focusedPane);
  focusedPaneRef.current = focusedPane;
  // A collapsed pane stays mounted (editor state, scroll and live output
  // survive) but is not rendered. Jumping into it restores it first, so every
  // existing focus command keeps working without knowing about collapse.
  const restoreCollapsedPane = useCallback((pane: DesktopPane | null): boolean => {
    if (!pane) return false;
    // Compact Desktop presents Sessions as a drawer whatever the wide-layout
    // preference says; reaching into it must not rewrite that preference.
    if (
      pane === "sessions" &&
      globalThis.matchMedia?.(DESKTOP_COMPACT_WIDTH_QUERY).matches
    ) return false;
    // Collapsed Sessions still has a visible, navigable rail: jumping there
    // focuses the rail and keeps the layout. Only `[` unfolds the list.
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
    const element = document.querySelector<HTMLElement>(
      `[data-desktop-region="${CSS.escape(region)}"]`,
    );
    if (!element) return;
    const pane = paneFromTarget(element);
    if (restoreCollapsedPane(pane)) {
      afterLayout(() => focusRegion(region));
      return;
    }
    if (pane) setFocusedPane(pane);
    setFocusedRegion(region);
    focusElement(element);
  }, [restoreCollapsedPane]);
  const focusPane = useCallback((pane: DesktopPane): void => {
    if (restoreCollapsedPane(pane)) {
      afterLayout(() => focusPane(pane));
      return;
    }
    setFocusedPane(pane);
    const paneElement = document.querySelector<HTMLElement>(`[data-desktop-pane="${pane}"]`);
    const region = paneElement?.querySelector<HTMLElement>("[data-desktop-region]");
    if (region?.dataset.desktopRegion) {
      setFocusedRegion(region.dataset.desktopRegion);
      focusElement(region);
    } else {
      focusElement(paneElement ?? null);
    }
  }, [restoreCollapsedPane]);
  const focusAdjacentPane = useCallback((delta: -1 | 1): void => {
    const order: DesktopPane[] = ["sessions", "prompt", "conversation"];
    // H/L is spatial movement across what is on screen; it never restores a
    // collapsed pane. Explicit jumps (workspace prefix S/P/C) do.
    const available = order.filter((pane) =>
      !collapsedPanes[pane] &&
      document.querySelector(`[data-desktop-pane="${pane}"]`)
    );
    if (available.length === 0) return;
    const current = Math.max(0, available.indexOf(focusedPane));
    focusPane(available[(current + delta + available.length) % available.length] as DesktopPane);
  }, [collapsedPanes, focusPane, focusedPane]);
  const togglePane = useCallback((pane: DesktopPane): void => {
    if (pane === "sessions" && globalThis.matchMedia?.(DESKTOP_COMPACT_WIDTH_QUERY).matches) {
      globalThis.dispatchEvent(new CustomEvent(DESKTOP_SESSIONS_DRAWER_TOGGLE_EVENT));
      return;
    }
    const current = desktopCollapsedPanesStore.get();
    const next = togglePaneCollapsed(current, pane);
    desktopCollapsedPanesStore.set(next);
    if (!next[pane]) {
      // Restoring is an explicit request for that pane: move into it.
      afterLayout(() => focusRegion(PANE_ENTRY_REGION[pane]));
      return;
    }
    const active = document.activeElement;
    const paneElement = document.querySelector(`[data-desktop-pane="${pane}"]`);
    const focusInside = focusedPaneRef.current === pane ||
      (active instanceof Node && paneElement?.contains(active) === true);
    // A swap (hiding the last work pane) always lands in the pane it revealed.
    const swapped = (["prompt", "conversation"] as const).find((other) =>
      other !== pane && current[other] && !next[other]
    );
    if (!focusInside && !swapped) return;
    const target: DesktopPane = swapped ??
      (pane === "prompt" ? "conversation" : next.prompt ? "conversation" : "prompt");
    afterLayout(() => focusRegion(PANE_ENTRY_REGION[target]));
  }, [focusRegion]);
  const focusAdjacentRegion = useCallback((delta: -1 | 1): void => {
    const next = verticalWorkspaceRegion(focusedPane, focusedRegion, delta);
    if (next && document.querySelector(`[data-desktop-region="${CSS.escape(next)}"]`)) {
      focusRegion(next);
    }
  }, [focusRegion, focusedPane, focusedRegion]);
  const cycleRegion = useCallback((): void => {
    const regions = [...document.querySelectorAll<HTMLElement>("[data-desktop-region]")]
      .filter((element) => element.offsetParent !== null);
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
        setSelectedSplitter(
          splitter === "sessions-prompt" || splitter === "prompt-conversation" ||
              splitter === "questions-page"
            ? splitter
            : null,
        );
      }
      const pane = paneFromTarget(event.target);
      if (pane) setFocusedPane(pane);
      const region = desktopRegionFromPointerTarget(event.target) ??
        regionFromTarget(event.target);
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
          const regionElement = document.querySelector<HTMLElement>(
            `[data-desktop-region="${CSS.escape(region)}"]`,
          );
          if (regionElement) focusElement(regionElement);
        } else if (document.activeElement instanceof HTMLElement) {
          document.activeElement.blur();
        }
      }
      if (region) {
        setFocusedRegion(region);
        // The default region state can be established before its lazy Desktop
        // subtree mounts. Update the marker in the input event as well as the
        // state effect so the very first focus reveals contextual keycaps.
        for (const element of document.querySelectorAll<HTMLElement>("[data-desktop-region]")) {
          if (element.dataset.desktopRegion === region) {
            element.dataset.desktopFocused = "true";
          } else {
            delete element.dataset.desktopFocused;
          }
        }
      }
      if (pane || region === "topbar.controls") {
        syncDesktopPaneChrome(
          pane ?? "prompt",
          region === "topbar.controls" ? "topbar.controls" : region,
        );
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
  }, [collapsedPanes, productMode, readingSidebarOpen, selectedSplitter]);

  useEffect(() => {
    const syncMountedWorkspace = (): boolean => {
      syncDesktopPaneChrome(focusedPane, focusedRegion);
      let focusedElement: HTMLElement | null = null;
      for (const element of document.querySelectorAll<HTMLElement>("[data-desktop-region]")) {
        if (element.dataset.desktopRegion === focusedRegion) {
          element.dataset.desktopFocused = "true";
          focusedElement = element;
        } else {
          delete element.dataset.desktopFocused;
        }
      }
      if (!focusedElement) return false;
      if (
        focusedElement.dataset.desktopRegion === "prompt.composer" &&
        !focusedElement.querySelector(
          "[data-vim-command-sink], .cm-content[contenteditable='true']",
        )
      ) {
        // The region shell arrived before its real editor. Keep observing past
        // the disabled preload mount; focusing the Paper here would strand the
        // eventual Vim command sink without a block cursor.
        return false;
      }
      // The provider's default state can predate the lazy Desktop subtree. Once
      // that region appears, make DOM focus agree with the status line only if
      // nobody else has claimed focus in the meantime.
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
    productMode,
    setProductMode,
    readingSidebarOpen,
    setReadingSidebarOpen,
    selectedSplitter,
    setSelectedSplitter,
    collapsedPanes,
    togglePane,
  }), [
    collapsedPanes,
    togglePane,
    cycleRegion,
    focusAdjacentPane,
    focusAdjacentRegion,
    focusPane,
    focusRegion,
    focusedPane,
    focusedRegion,
    mode,
    productMode,
    readingSidebarOpen,
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
