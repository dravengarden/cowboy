import type { Mode as ThemeMode } from "../theme";
import { MobileProductShell } from "./shell/MobileProductShell";

export function MobileApp({
  themeMode,
  onSetThemeMode,
}: {
  themeMode: ThemeMode;
  onSetThemeMode: (mode: ThemeMode) => void;
}): React.JSX.Element {
  return <MobileProductShell themeMode={themeMode} onSetThemeMode={onSetThemeMode} />;
}
