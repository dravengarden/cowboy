// cowboy's MUI theme. The dark/light/system *selection* is shared (app-shell
// SDK's useThemeMode — persistence + OS resolution); this file only builds the
// theme object and the status-bar colour from the resolved mode.
//
// Palette: soft lavender / violet. Light mode picks the same dusty-lavender
// background tone as Chrome's tab bar on macOS (the visual reference the
// user pointed at), with a slightly deeper violet for primary actions so
// buttons / user message bubbles read as "purple" but not garish. Dark mode
// is a deep purple-black that still reads as purple rather than black.

import { useEffect, useMemo } from "react";
import { createTheme, type Theme } from "@mui/material";

import { type ThemeChoice, useThemeMode as useSharedThemeMode } from "./_shell";

// cowboy's selection surface (Settings dialog, theme toggle) speaks the same
// system/light/dark vocabulary as the shared hook.
export type Mode = ThemeChoice;

// MUI AppBar color="default" resolves to grey[100]/grey[900]; mirror it onto
// the theme-color meta so the iOS standalone status bar tracks the active
// theme (status-bar-style="default" lets iOS tint the bar + auto-contrast its
// glyphs).
function applyThemeColor(dark: boolean): void {
  globalThis.document
    .querySelector('meta[name="theme-color"]')
    ?.setAttribute("content", dark ? "#15111d" : "#f4ecf7");
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
  const { choice, resolved, setChoice, cycle } = useSharedThemeMode("cowboy");
  const dark = resolved === "dark";
  useEffect(() => {
    applyThemeColor(dark);
  }, [dark]);

  const theme = useMemo(
    () =>
      createTheme({
        components: {
          // Touch ergonomics (ui.md §7): on a coarse pointer no interactive
          // control drops below the ~40px tap-target floor, even when size="small"
          // is asked for desktop density — "mobile never small". Desktop keeps it.
          MuiIconButton: { styleOverrides: { sizeSmall: { "@media (pointer: coarse)": { width: 40, height: 40 } } } },
          MuiButton: { styleOverrides: { sizeSmall: { "@media (pointer: coarse)": { minHeight: 40 } } } },
          MuiToggleButton: { styleOverrides: { sizeSmall: { "@media (pointer: coarse)": { minHeight: 40, minWidth: 40 } } } },
        },
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

  return { theme, mode: choice, setMode: setChoice, cycle };
}
