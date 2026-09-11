// cowboy's MUI theme. The dark/light/system *selection* is shared (app-shell
// SDK's useThemeMode — persistence + OS resolution); this file only builds the
// theme object and the status-bar colour from the resolved mode.
//
// Palette: each curated style owns its light and dark colors. Light mode is the product's
// default: a quiet cool-gray canvas, white work surfaces, and a deep blue
// action colour. The separation matters more than a dramatic tint — the
// transcript, composer, and tool cards should read as three useful layers.
// Dark mode remains available as an explicit preference.

import { useEffect, useMemo, useSyncExternalStore } from "react";
import { alpha, createTheme, type Theme } from "@mui/material";
import { currentAppIcon, DEFAULT_APP_ICON, subscribeAppIcon } from "./appIcons";
import { appearancePalette } from "./appearanceThemes";

import {
  type ThemeChoice,
  useThemeMode as useSharedThemeMode,
} from "@cowboy/app-shell";
import {
  COARSE_POINTER_ROOT_CLASS,
  prefersCoarsePointer,
  syncCoarsePointerRootClass,
  syncPhoneStandaloneRootClass,
} from "./platform";
import { browserTooltipListenerPolicy } from "./tooltipPolicy";

// cowboy's selection surface (Settings dialog, theme toggle) speaks the same
// system/light/dark vocabulary as the shared hook.
export type Mode = ThemeChoice;

// Opaque by design: translucent focus colors look different over Composer
// paper and Draft `action.selected` even when their alpha is identical.
export function desktopFocusBoundary(theme: Theme): string {
  const weight = theme.palette.mode === "dark" ? 58 : 48;
  return `color-mix(in srgb, ${theme.palette.primary.main} ${
    String(weight)
  }%, ${theme.palette.background.default})`;
}

// One quiet fill for every Desktop workspace target. The boundary carries the
// focus information; this tint only groups the active surface, so it stays
// deliberately subtle even on the full-height Composer canvas.
export function desktopFocusFill(theme: Theme): string {
  return alpha(
    theme.palette.primary.main,
    theme.palette.mode === "dark" ? 0.075 : 0.045,
  );
}

// Keep the iOS standalone status bar in lockstep with the navbar surface. The
// AppBar is pinned to the selected style's `background.default`, so the
// theme-color meta uses the SAME value: status bar →
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
function applyThemeColor(color: string): void {
  const doc = globalThis.document;
  if (!doc) return;
  for (const m of doc.querySelectorAll('meta[name="theme-color"]')) m.remove();
  const meta = doc.createElement("meta");
  meta.setAttribute("name", "theme-color");
  meta.setAttribute("content", color);
  doc.head.appendChild(meta);
  // iOS paints the unlaid-out strip under a rising keyboard from the
  // document background. Keep it on the app surface so that frame is
  // not a black/white flash.
  doc.documentElement.style.backgroundColor = color;
  if (doc.body) doc.body.style.backgroundColor = color;
}

// Keep the first visit calm and readable even when the host OS is in dark
// mode. Settings can still opt into System or Dark; this only seeds the
// preference when Cowboy has never stored one before. In particular, do not
// overwrite an existing choice made by the user on a later visit.
function seedLightThemeDefault(): void {
  try {
    if (globalThis.localStorage.getItem("cowboy-theme-mode") === null) {
      globalThis.localStorage.setItem("cowboy-theme-mode", "light");
    }
  } catch {
    // Private browsing / disabled storage: the shared hook will fall back to
    // its normal system behaviour without making theme selection fatal.
  }
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
syncCoarsePointerRootClass();
syncPhoneStandaloneRootClass();

export interface ThemeControls {
  theme: Theme;
  mode: Mode;
  setMode: (m: Mode) => void;
  /** Cycle system → light → dark → system. Kept for the legacy single-button
   *  call site; new code should use `setMode` directly. */
  cycle: () => void;
}

export function useThemeMode(): ThemeControls {
  seedLightThemeDefault();
  const { choice, resolved, setChoice, cycle } = useSharedThemeMode("cowboy");
  const dark = resolved === "dark";
  const icon = useSyncExternalStore(
    subscribeAppIcon,
    currentAppIcon,
    () => DEFAULT_APP_ICON,
  );
  const palette = useMemo(() => appearancePalette(icon, dark), [icon, dark]);
  useEffect(() => {
    syncCoarsePointerRootClass();
    syncPhoneStandaloneRootClass();
    applyThemeColor(palette.background.default);
    // An iOS standalone PWA latches the status-bar colour and IGNORES later
    // updates across a background→resume: leave the app in dark, switch away,
    // come back, and the bar is stuck on a stale (light) colour over a dark app
    // (the reported "top doesn't match the theme" bug). Re-assert whenever we
    // become visible again and on bfcache restore, so the bar always re-reads
    // the current mode. (Mirrors liveview's useTheme.)
    const reassert = (): void => applyThemeColor(palette.background.default);
    const onVisible = (): void => {
      if (globalThis.document?.visibilityState === "visible") reassert();
    };
    globalThis.addEventListener("visibilitychange", onVisible);
    globalThis.addEventListener("pageshow", reassert);
    return () => {
      globalThis.removeEventListener("visibilitychange", onVisible);
      globalThis.removeEventListener("pageshow", reassert);
    };
  }, [palette.background.default]);

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
          // iOS Safari and the WKWebView shell both paint the UA tap wash on
          // the element under the finger. Opening Session settings mounts the
          // sheet dismiss under that same point, so the wash lands on the
          // bottom chrome / close island and stays there. Kill it at the
          // document root before any control can inherit the default grey.
          MuiCssBaseline: {
            styleOverrides: {
              html: { WebkitTapHighlightColor: "transparent" },
              body: { WebkitTapHighlightColor: "transparent" },
              "#root": { WebkitTapHighlightColor: "transparent" },
            },
          },
          // Session-sheet dismiss is a ButtonBase, not an IconButton. Clear a
          // leftover hover/focus latch on unfilled controls, but retain the
          // semantic fill of contained actions: iOS keeps :hover after a tap,
          // and transparent + contrastText paints those buttons as blank bars.
          MuiButtonBase: {
            defaultProps: {
              disableRipple: prefersCoarsePointer(),
              disableTouchRipple: prefersCoarsePointer(),
            },
            styleOverrides: {
              root: {
                WebkitTapHighlightColor: "transparent",
                [`html.${COARSE_POINTER_ROOT_CLASS} &`]: {
                  "&:not(.MuiButton-contained):hover, &:not(.MuiButton-contained).Mui-focusVisible":
                    {
                      backgroundColor: "transparent",
                    },
                },
                "@media (hover: none), (pointer: coarse), (any-pointer: coarse)":
                  {
                    "&:not(.MuiButton-contained):hover, &:not(.MuiButton-contained).Mui-focusVisible":
                      {
                        backgroundColor: "transparent",
                      },
                  },
              },
            },
          },
          // Touch ergonomics (ui.md §7): on a coarse pointer no interactive
          // control drops below the ~40px tap-target floor, even when size="small"
          // is asked for desktop density — "mobile never small". Desktop keeps it.
          // Unify every icon button to the large session-list TAP TARGET (44px),
          // while the GLYPH stays 1.5rem (MUI "medium") so it scales WITH the
          // reading font like the rest of the UI — a big box, a font-tracking
          // glyph. `1.5rem` also normalises the icons that asked for `small`
          // (1.25rem) up to one size, and the text "/" skills glyph (1.375rem) is
          // tuned to match it. An instance can override size + its own
          // `& .MuiSvgIcon-root` rule via sx (e.g. the compact copy button).
          MuiIconButton: {
            defaultProps: {
              // Overlay dismiss can leave Mui-focusVisible + the color=primary
              // focus ripple latched on the control that was under the finger
              // (Session settings × sits over the composer send/queue button).
              disableFocusRipple: prefersCoarsePointer(),
            },
            styleOverrides: {
              root: {
                width: 44,
                height: 44,
                "& .MuiSvgIcon-root": { fontSize: "1.5rem" },
                // WebKit synthesizes hover/focus after a finger tap. A later
                // unscoped MUI v6 color variant sets --IconButton-hoverBg and
                // wins a same-specificity media-query reset; iOS can also flip
                // the primary pointer to `fine`/`hover` after the first tap so
                // `(pointer: coarse)` stops matching. Pin the kill to the
                // document class (snapshotted at load) and beat color=primary
                // with an extra class.
                [`html.${COARSE_POINTER_ROOT_CLASS} &`]: {
                  "--IconButton-hoverBg": "transparent",
                  "&.MuiIconButton-root.MuiIconButton-colorPrimary, &.MuiIconButton-root.MuiIconButton-colorSecondary, &.MuiIconButton-root.MuiIconButton-colorError, &.MuiIconButton-root.MuiIconButton-colorInfo, &.MuiIconButton-root.MuiIconButton-colorSuccess, &.MuiIconButton-root.MuiIconButton-colorWarning":
                    {
                      "--IconButton-hoverBg": "transparent",
                    },
                  "&:hover, &.Mui-focusVisible": {
                    backgroundColor: "transparent",
                  },
                  "&:not(.Mui-selected):active": {
                    backgroundColor: "action.selected",
                  },
                },
                "@media (hover: none), (pointer: coarse), (any-pointer: coarse)":
                  {
                    "--IconButton-hoverBg": "transparent",
                    "&:not(.Mui-selected):hover, &:not(.Mui-selected).Mui-focusVisible":
                      {
                        backgroundColor: "transparent",
                      },
                    "&:not(.Mui-selected):active": {
                      backgroundColor: "action.selected",
                    },
                  },
              },
            },
          },
          // MUI's Button start/end-icon rules use fixed 18px/20px glyphs by
          // default. That breaks Cowboy's global font-size contract: labels grow
          // while their action glyphs stay behind. Own the child size with rem so
          // every ordinary Button icon follows the root scale. The physical tap
          // target remains independently bounded below on touch surfaces.
          MuiButton: {
            styleOverrides: {
              root: {
                "& .MuiButton-startIcon.MuiButton-icon > :nth-of-type(1), & .MuiButton-endIcon.MuiButton-icon > :nth-of-type(1)":
                  {
                    fontSize: "1.25rem",
                  },
              },
              sizeSmall: {
                "& .MuiButton-startIcon.MuiButton-icon > :nth-of-type(1), & .MuiButton-endIcon.MuiButton-icon > :nth-of-type(1)":
                  {
                    fontSize: "1.125rem",
                  },
                "@media (pointer: coarse)": { minHeight: 40 },
              },
            },
          },
          MuiToggleButton: {
            styleOverrides: {
              sizeSmall: {
                "@media (pointer: coarse)": { minHeight: 40, minWidth: 40 },
              },
            },
          },
          // Tooltips are a DESKTOP-HOVER affordance only. On a touch screen MUI
          // fires them on tap-focus AND long-press, and they LINGER — tapping any
          // icon button focuses it and pops a bubble that's hard to dismiss (the
          // reported stuck "Rename session" tooltip). Disable the focus + touch
          // triggers globally. iOS also synthesizes mouse hover after a tap, so
          // disable the hover listener unless the primary pointer can genuinely
          // hover. Every control carries an `aria-label`, so no information is
          // lost on touch / for assistive tech. Desktop mouse hover stays intact.
          MuiTooltip: {
            defaultProps: browserTooltipListenerPolicy(),
          },
          // (No selected-MenuItem override — the solid primary fill read as heavy;
          // a selected item is marked by its ✓ checkmark + MUI's default subtle
          // `action.selected` tint, which is enough.)
        },
        palette,
        shape: { borderRadius: 10 },
      }),
    [dark, palette],
  );

  return { theme, mode: choice, setMode: setChoice, cycle };
}
