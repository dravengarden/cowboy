// Browser conformance uses the same overrides and pointer policy as the SPA.
import type { ReactNode } from "react";
import { ThemeProvider } from "@mui/material";
import { useThemeMode } from "./theme";

export function BrowserProductTheme(
  { children }: { children: ReactNode },
): React.JSX.Element {
  const { theme } = useThemeMode();
  return <ThemeProvider theme={theme}>{children}</ThemeProvider>;
}
