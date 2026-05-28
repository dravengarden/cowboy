// Dark/light/system theme toggle, persisted to localStorage (omega's recipe).

import { useMemo, useState } from "react";
import { createTheme, useMediaQuery, type Theme } from "@mui/material";

type Mode = "system" | "light" | "dark";
const STORAGE_KEY = "cowboy-theme-mode";

function loadMode(): Mode {
  const v = globalThis.localStorage.getItem(STORAGE_KEY);
  return v === "light" || v === "dark" || v === "system" ? v : "system";
}

function nextMode(m: Mode): Mode {
  if (m === "system") return "light";
  if (m === "light") return "dark";
  return "system";
}

export interface ThemeControls {
  theme: Theme;
  mode: Mode;
  cycle: () => void;
}

export function useThemeMode(): ThemeControls {
  const [mode, setMode] = useState<Mode>(loadMode);
  const systemDark = useMediaQuery("(prefers-color-scheme: dark)");
  const dark = mode === "dark" || (mode === "system" && systemDark);

  const theme = useMemo(() => createTheme({ palette: { mode: dark ? "dark" : "light" } }), [dark]);

  function cycle(): void {
    setMode((current) => {
      const next = nextMode(current);
      globalThis.localStorage.setItem(STORAGE_KEY, next);
      return next;
    });
  }

  return { theme, mode, cycle };
}
