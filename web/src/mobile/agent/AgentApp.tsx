import { App } from "../../App";
import type { Mode as ThemeMode } from "../../theme";

export function AgentApp({
  themeMode,
  onSetThemeMode,
}: {
  themeMode: ThemeMode;
  onSetThemeMode: (mode: ThemeMode) => void;
}): React.JSX.Element {
  return (
    <App
      themeMode={themeMode}
      onSetThemeMode={onSetThemeMode}
      surface="touch"
    />
  );
}

