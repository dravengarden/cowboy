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
  focusPane: (pane: DesktopPane) => void;
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

export function DesktopWorkspaceProvider({
  children,
}: {
  children: React.ReactNode;
}): React.JSX.Element {
  const [focusedPane, setFocusedPane] = useState<DesktopPane>("prompt");
  const [mode, setMode] = useState<WorkspaceMode>("normal");
  const focusPane = useCallback((pane: DesktopPane): void => setFocusedPane(pane), []);

  useEffect(() => {
    const syncPane = (event: Event): void => {
      const pane = paneFromTarget(event.target);
      if (pane) setFocusedPane(pane);
    };
    document.addEventListener("pointerdown", syncPane, true);
    document.addEventListener("focusin", syncPane, true);
    return () => {
      document.removeEventListener("pointerdown", syncPane, true);
      document.removeEventListener("focusin", syncPane, true);
    };
  }, []);

  const value = useMemo<DesktopWorkspaceContextValue>(() => ({
    focusedPane,
    focusPane,
    mode,
    setMode,
  }), [focusPane, focusedPane, mode]);

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
