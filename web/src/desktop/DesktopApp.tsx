import { useEffect } from "react";
import { App } from "../App";
import type { Mode as ThemeMode } from "../theme";
import { DesktopCommandProvider } from "./commands/DesktopCommandProvider";
import { DesktopWorkspaceProvider } from "./DesktopWorkspaceController";

export function DesktopApp({
  themeMode,
  onSetThemeMode,
}: {
  themeMode: ThemeMode;
  onSetThemeMode: (mode: ThemeMode) => void;
}): React.JSX.Element {
  useEffect(() => {
    (globalThis as typeof globalThis & {
      __cowboyDesktopBootReady?: () => void;
    }).__cowboyDesktopBootReady?.();
  }, []);
  return (
    <DesktopWorkspaceProvider>
      <DesktopCommandProvider>
        <App
          themeMode={themeMode}
          onSetThemeMode={onSetThemeMode}
          surface="desktop"
        />
      </DesktopCommandProvider>
    </DesktopWorkspaceProvider>
  );
}
