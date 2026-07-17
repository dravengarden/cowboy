import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
} from "react";

export type DesktopPane = "sessions" | "prompt" | "conversation";
export type WorkspaceMode = "normal" | "hint" | "search" | "command";

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
  // A contextual Editor jump is navigation, not an Insert command. Prefer the
  // Desktop Vim command sink so it lands explicitly in Normal; when Vim is off,
  // fall back to the native CodeMirror content surface.
  (composerCommandSink ?? composer ?? currentItem ?? preferred ?? firstItem ?? planToggle ?? element).focus({
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
  const focusRegion = useCallback((region: string): void => {
    const element = document.querySelector<HTMLElement>(
      `[data-desktop-region="${CSS.escape(region)}"]`,
    );
    if (!element) return;
    const pane = paneFromTarget(element);
    if (pane) setFocusedPane(pane);
    setFocusedRegion(region);
    focusElement(element);
  }, []);
  const focusPane = useCallback((pane: DesktopPane): void => {
    setFocusedPane(pane);
    const paneElement = document.querySelector<HTMLElement>(`[data-desktop-pane="${pane}"]`);
    const region = paneElement?.querySelector<HTMLElement>("[data-desktop-region]");
    if (region?.dataset.desktopRegion) {
      setFocusedRegion(region.dataset.desktopRegion);
      focusElement(region);
    } else {
      focusElement(paneElement ?? null);
    }
  }, []);
  const focusAdjacentPane = useCallback((delta: -1 | 1): void => {
    const order: DesktopPane[] = ["sessions", "prompt", "conversation"];
    const available = order.filter((pane) =>
      document.querySelector(`[data-desktop-pane="${pane}"]`)
    );
    if (available.length === 0) return;
    const current = Math.max(0, available.indexOf(focusedPane));
    focusPane(available[(current + delta + available.length) % available.length] as DesktopPane);
  }, [focusPane, focusedPane]);
  const focusAdjacentRegion = useCallback((delta: -1 | 1): void => {
    const pane = document.querySelector<HTMLElement>(`[data-desktop-pane="${focusedPane}"]`);
    const regions = [...(pane?.querySelectorAll<HTMLElement>("[data-desktop-region]") ?? [])]
      .filter((element) => element.offsetParent !== null);
    if (regions.length === 0) return;
    const current = Math.max(
      0,
      regions.findIndex((element) => element.dataset.desktopRegion === focusedRegion),
    );
    const next = regions[(current + delta + regions.length) % regions.length];
    if (next?.dataset.desktopRegion) focusRegion(next.dataset.desktopRegion);
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
      const pane = paneFromTarget(event.target);
      if (pane) {
        setFocusedPane(pane);
        for (const element of document.querySelectorAll<HTMLElement>("[data-desktop-pane]")) {
          if (element.dataset.desktopPane === pane) {
            element.dataset.desktopPaneFocused = "true";
          } else {
            delete element.dataset.desktopPaneFocused;
          }
        }
      }
      const region = regionFromTarget(event.target);
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
    };
    document.addEventListener("pointerdown", syncPane, true);
    document.addEventListener("focusin", syncPane, true);
    return () => {
      document.removeEventListener("pointerdown", syncPane, true);
      document.removeEventListener("focusin", syncPane, true);
    };
  }, []);

  useEffect(() => {
    const syncMountedWorkspace = (): boolean => {
      for (const element of document.querySelectorAll<HTMLElement>("[data-desktop-pane]")) {
        if (element.dataset.desktopPane === focusedPane) {
          element.dataset.desktopPaneFocused = "true";
        } else {
          delete element.dataset.desktopPaneFocused;
        }
      }
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
  }), [
    cycleRegion,
    focusAdjacentPane,
    focusAdjacentRegion,
    focusPane,
    focusRegion,
    focusedPane,
    focusedRegion,
    mode,
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
