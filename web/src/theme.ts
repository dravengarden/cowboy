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
//
// REPLACE the <meta> node rather than mutate its `content`: an iOS standalone
// PWA latches the status-bar colour from the theme-color meta and routinely
// IGNORES a later `setAttribute` on the same node, so a live dark→light switch
// left the status bar stuck on the load-time (dark) colour. Removing the node
// and appending a fresh one forces iOS to re-read it. Harmless elsewhere —
// every other browser honours either path.
function applyThemeColor(dark: boolean): void {
  const doc = globalThis.document;
  if (!doc) return;
  for (const m of doc.querySelectorAll('meta[name="theme-color"]')) m.remove();
  const meta = doc.createElement("meta");
  meta.setAttribute("name", "theme-color");
  meta.setAttribute("content", dark ? "#15111d" : "#f4ecf7");
  doc.head.appendChild(meta);
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
    // An iOS standalone PWA latches the status-bar colour and IGNORES later
    // updates across a background→resume: leave the app in dark, switch away,
    // come back, and the bar is stuck on a stale (light) colour over a dark app
    // (the reported "top doesn't match the theme" bug). Re-assert whenever we
    // become visible again and on bfcache restore, so the bar always re-reads
    // the current mode. (Mirrors liveview's useTheme.)
    const reassert = (): void => applyThemeColor(dark);
    const onVisible = (): void => {
      if (globalThis.document?.visibilityState === "visible") reassert();
    };
    globalThis.addEventListener("visibilitychange", onVisible);
    globalThis.addEventListener("pageshow", reassert);
    return () => {
      globalThis.removeEventListener("visibilitychange", onVisible);
      globalThis.removeEventListener("pageshow", reassert);
    };
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
          // Unify EVERY icon button to the large session-list size (44px) with a
          // FIXED 24px glyph — consistent tap targets, and the glyph no longer
          // shrinks with the reading font scale (icons are otherwise rem-based,
          // so at 85% they read tiny). An instance that needs a different size
          // sets width/height (+ its own `& .MuiSvgIcon-root` rule) via sx, which
          // wins (e.g. the compact code-block copy button).
          MuiIconButton: {
            styleOverrides: {
              root: {
                width: 44,
                height: 44,
                "& .MuiSvgIcon-root": { fontSize: 24 },
              },
            },
          },
          MuiButton: { styleOverrides: { sizeSmall: { "@media (pointer: coarse)": { minHeight: 40 } } } },
          MuiToggleButton: { styleOverrides: { sizeSmall: { "@media (pointer: coarse)": { minHeight: 40, minWidth: 40 } } } },
          // Tooltips are a DESKTOP-HOVER affordance only. On a touch screen MUI
          // fires them on tap-focus AND long-press, and they LINGER — tapping any
          // icon button focuses it and pops a bubble that's hard to dismiss (the
          // reported stuck "Rename session" tooltip). Disable the focus + touch
          // triggers globally so a tooltip shows on hover (desktop) and NEVER on
          // touch. Every control carries an `aria-label`, so no information is
          // lost on touch / for assistive tech. (Same trick the auto-scroll toggle
          // applied per-instance, made the default so every tooltip inherits it.)
          MuiTooltip: {
            defaultProps: { disableFocusListener: true, disableTouchListener: true },
          },
          // A SELECTED dropdown item uses the primary theme colour as a SOLID
          // fill, not the default faint ~12% action.selected tint (which read as
          // barely-there — the reported "淡紫色太不明显"). The current choice is now
          // unmistakable. Title + description inherit the contrast colour (the
          // description a touch dimmer for hierarchy); hover / keyboard-focus
          // deepen to primary.dark.
          MuiMenuItem: {
            styleOverrides: {
              root: ({ theme: t }) => ({
                "&.Mui-selected": {
                  backgroundColor: t.palette.primary.main,
                  color: t.palette.primary.contrastText,
                  "& .MuiTypography-root": { color: "inherit" },
                  "& .MuiTypography-caption": { opacity: 0.82 },
                  "&:hover, &.Mui-focusVisible, &.Mui-selected:hover": {
                    backgroundColor: t.palette.primary.dark,
                  },
                },
              }),
            },
          },
        },
        palette: dark
          ? {
              mode: "dark",
              // Refined Radix-Violet family (less neon than Tailwind violet): a
              // soft light step for the dark-mode bubble, violet-9 (#6E56CF) the
              // deep step.
              primary: {
                main: "#9E8CFC",
                light: "#C9BCFF",
                dark: "#6E56CF",
                contrastText: "#1c1428",
              },
              secondary: { main: "#f0abfc" }, // fuchsia-300 for accents
              background: {
                default: "#15111d", // deep purple-black
                paper: "#1f1a2c",
              },
              divider: "rgba(158, 140, 252, 0.18)",
              text: {
                primary: "#ede9fe",
                secondary: "#a899c4",
              },
              action: {
                hover: "rgba(158, 140, 252, 0.10)",
                selected: "rgba(158, 140, 252, 0.18)",
              },
            }
          : {
              mode: "light",
              // Radix Violet 9 (#6E56CF): the refined, slightly blue-shifted
              // accent — softer than Tailwind violet-600's neon while still dark
              // enough to carry white bubble/button text (Radix pairs violet-9
              // with white). 10/8 give the pressed + lighter steps.
              primary: {
                main: "#6E56CF", // the "send" / user-bubble tone
                light: "#8B79E0",
                dark: "#5B4BC4",
                contrastText: "#ffffff",
              },
              secondary: { main: "#c026d3" }, // fuchsia accent
              background: {
                // The Chrome-tab-bar lavender the user pointed at: a desaturated
                // pinkish violet that's calm on the eyes for long sessions.
                default: "#f4ecf7",
                // Paper is the ELEVATED surface (cards, dialogs, the DetentSheet):
                // lighter than `default` so it reads as raised, but a lavender-
                // tinted white rather than near-pure-white — a stark #fdfbff read
                // as a harsh white box over the soft lavender page.
                paper: "#faf6fd",
              },
              divider: "rgba(110, 86, 207, 0.18)",
              text: {
                primary: "#1c1428",
                secondary: "#6b5e80",
              },
              action: {
                hover: "rgba(110, 86, 207, 0.06)",
                selected: "rgba(110, 86, 207, 0.12)",
              },
            },
        shape: { borderRadius: 10 },
      }),
    [dark],
  );

  return { theme, mode: choice, setMode: setChoice, cycle };
}
