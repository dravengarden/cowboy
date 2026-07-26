import { App } from "../../App";
import type { Mode as ThemeMode } from "../../theme";

export function AgentApp({
  themeMode,
  onSetThemeMode,
  onDrawerOpenChange,
}: {
  themeMode: ThemeMode;
  onSetThemeMode: (mode: ThemeMode) => void;
  onDrawerOpenChange: (open: boolean) => void;
}): React.JSX.Element {
  return (
    <App
      themeMode={themeMode}
      onSetThemeMode={onSetThemeMode}
      surface="touch"
      onMobileDrawerOpenChange={onDrawerOpenChange}
    />
  );
}
