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

// Keep the iOS standalone status bar in lockstep with the navbar surface. The
// AppBar is pinned to `background.default` (see App.tsx — `#15111d` dark /
// `#f4ecf7` light), so the theme-color meta uses the SAME values: status bar →
// navbar read as one surface (status-bar-style="default" lets iOS tint the bar
// + auto-contrast its glyphs). Must stay in sync with the palette's
// background.default below.
function applyThemeColor(dark: boolean): void {
  globalThis.document
    .querySelector('meta[name="theme-color"]')
    ?.setAttribute("content", dark ? "#15111d" : "#f4ecf7");
}

// Native desktop UIs size their system font per-OS: macOS renders SF at ~13px,
// Windows/Linux UIs sit a touch larger. The web default of 16px is a *document
// reading* size and looks oversized for an app chrome on macOS (the reference
// being native apps like Zed) — so pick the platform's native UI size and the
// panel reads like a native app, not a web page. Touch (iOS/iPad) stays at 16:
// it's the right reading size for a phone, and < 16px on inputs triggers iOS's
// focus auto-zoom. Computed once at module load — platform doesn't change mid-
// session — and the `system-ui` font stack already matches each OS's UI face.
function osBaseFontSize(): number {
  const nav = globalThis.navigator as
    | (Navigator & { userAgentData?: { platform?: string } })
    | undefined;
  if (globalThis.matchMedia?.("(pointer: coarse)").matches) return 16;
  const ua = nav?.userAgent ?? "";
  const platform = nav?.userAgentData?.platform ?? nav?.platform ?? "";
  if (/mac/i.test(platform) || /Macintosh/i.test(ua)) return 13;
  if (/win/i.test(platform) || /Windows/i.test(ua)) return 14;
  return 14; // Linux / other — GTK/Qt UIs sit around 14–15px
}

const OS_BASE_FONT_SIZE = osBaseFontSize();

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
        typography: {
          // Use the OS UI font + the OS default body size (16px), not MUI's
          // bundled-Roboto default. Roboto isn't shipped here, so the default
          // silently fell back to Helvetica/Arial — visibly NOT the system font,
          // and a font-swap flash against index.html's `-apple-system` splash.
          // This stack matches the splash and cmTheme.ts so mount is seamless.
          // Base size follows the OS's native UI size (osBaseFontSize) instead of
          // the one-size-fits-all 16px, which read oversized as app chrome on
          // macOS. MUI's base coefficient defaults to 14.
          fontFamily:
            'system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue", Arial, "Noto Sans", sans-serif',
          fontSize: OS_BASE_FONT_SIZE,
        },
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
