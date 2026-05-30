// cowboy's MUI theme. The dark/light/system selection is shared (app-shell
// SDK's useThemeMode); this only builds the theme object + the status-bar
// colour from the resolved mode.

import { useEffect, useMemo } from "react";
import { createTheme, type Theme } from "@mui/material";

import { type ThemeChoice, useThemeMode as useSharedThemeMode } from "./_shell";

// MUI AppBar color="default" resolves to grey[100]/grey[900]; mirror it onto
// the theme-color meta so the iOS standalone status bar tracks the active
// theme (status-bar-style="default" lets iOS tint the bar + auto-contrast its
// glyphs).
function applyThemeColor(dark: boolean): void {
  globalThis.document
    .querySelector('meta[name="theme-color"]')
    ?.setAttribute("content", dark ? "#212121" : "#f5f5f5");
}

export interface ThemeControls {
  theme: Theme;
  mode: ThemeChoice;
  cycle: () => void;
}

export function useThemeMode(): ThemeControls {
  const { choice, resolved, cycle } = useSharedThemeMode("cowboy");
  const dark = resolved === "dark";
  useEffect(() => {
    applyThemeColor(dark);
  }, [dark]);

  const theme = useMemo(() => createTheme({ palette: { mode: resolved } }), [resolved]);

  return { theme, mode: choice, cycle };
}
