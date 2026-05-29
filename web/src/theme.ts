// Dark/light/system theme toggle, persisted to localStorage (omega's recipe).
//
// Palette: soft lavender / violet. Light mode picks the same dusty-lavender
// background tone as Chrome's tab bar on macOS (the visual reference the
// user pointed at), with a slightly deeper violet for primary actions so
// buttons / user message bubbles read as "purple" but not garish. Dark mode
// is a deep purple-black that still reads as purple rather than black.

import { useMemo, useState, useCallback } from "react";
import { createTheme, useMediaQuery, type Theme } from "@mui/material";

export type Mode = "system" | "light" | "dark";
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
  setMode: (m: Mode) => void;
  /** Cycle system → light → dark → system. Kept for the legacy single-button
   *  call site; new code should use `setMode` directly. */
  cycle: () => void;
}

export function useThemeMode(): ThemeControls {
  const [mode, setMode] = useState<Mode>(loadMode);
  const systemDark = useMediaQuery("(prefers-color-scheme: dark)");
  const dark = mode === "dark" || (mode === "system" && systemDark);

  const theme = useMemo(
    () =>
      createTheme({
        palette: dark
          ? {
              mode: "dark",
              primary: {
                main: "#a78bfa", // violet-400
                light: "#c4b5fd",
                dark: "#7c3aed",
                contrastText: "#1c1428",
              },
              secondary: { main: "#f0abfc" }, // fuchsia-300 for accents
              background: {
                default: "#15111d", // deep purple-black
                paper: "#1f1a2c",
              },
              divider: "rgba(167, 139, 250, 0.18)",
              text: {
                primary: "#ede9fe",
                secondary: "#a899c4",
              },
              action: {
                hover: "rgba(167, 139, 250, 0.10)",
                selected: "rgba(167, 139, 250, 0.18)",
              },
            }
          : {
              mode: "light",
              primary: {
                main: "#7c3aed", // violet-600 — the "send" / user-bubble tone
                light: "#a78bfa",
                dark: "#6d28d9",
                contrastText: "#ffffff",
              },
              secondary: { main: "#c026d3" }, // fuchsia accent
              background: {
                // The Chrome-tab-bar lavender the user pointed at: a desaturated
                // pinkish violet that's calm on the eyes for long sessions.
                default: "#f4ecf7",
                paper: "#fdfbff",
              },
              divider: "rgba(124, 58, 237, 0.18)",
              text: {
                primary: "#1c1428",
                secondary: "#6b5e80",
              },
              action: {
                hover: "rgba(124, 58, 237, 0.06)",
                selected: "rgba(124, 58, 237, 0.12)",
              },
            },
        shape: { borderRadius: 10 },
      }),
    [dark],
  );

  const setModePersist = useCallback((next: Mode): void => {
    setMode(next);
    globalThis.localStorage.setItem(STORAGE_KEY, next);
  }, []);

  function cycle(): void {
    setMode((current) => {
      const next = nextMode(current);
      globalThis.localStorage.setItem(STORAGE_KEY, next);
      return next;
    });
  }

  return { theme, mode, setMode: setModePersist, cycle };
}
