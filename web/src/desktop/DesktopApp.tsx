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
