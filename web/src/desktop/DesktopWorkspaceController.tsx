import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
} from "react";

export type DesktopPane = "sessions" | "prompt" | "conversation";
export type WorkspaceMode = "normal" | "leader" | "hint" | "search" | "command";

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
  leaderPrefix: string[];
  setLeaderPrefix: (prefix: string[]) => void;
  leaderMessage: string | null;
  setLeaderMessage: (message: string | null) => void;
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
  const preferred = element.querySelector<HTMLElement>("[data-desktop-focus-default]");
  (preferred ?? element).focus({ preventScroll: true });
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
  const [leaderPrefix, setLeaderPrefix] = useState<string[]>([]);
  const [leaderMessage, setLeaderMessage] = useState<string | null>(null);
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
      if (pane) setFocusedPane(pane);
      const region = regionFromTarget(event.target);
      if (region) setFocusedRegion(region);
    };
    document.addEventListener("pointerdown", syncPane, true);
    document.addEventListener("focusin", syncPane, true);
    return () => {
      document.removeEventListener("pointerdown", syncPane, true);
      document.removeEventListener("focusin", syncPane, true);
    };
  }, []);

  useEffect(() => {
    for (const element of document.querySelectorAll<HTMLElement>("[data-desktop-region]")) {
      if (element.dataset.desktopRegion === focusedRegion) {
        element.dataset.desktopFocused = "true";
      } else {
        delete element.dataset.desktopFocused;
      }
    }
  }, [focusedRegion]);

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
    leaderPrefix,
    setLeaderPrefix,
    leaderMessage,
    setLeaderMessage,
  }), [
    cycleRegion,
    focusAdjacentPane,
    focusAdjacentRegion,
    focusPane,
    focusRegion,
    focusedPane,
    focusedRegion,
    leaderMessage,
    leaderPrefix,
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
