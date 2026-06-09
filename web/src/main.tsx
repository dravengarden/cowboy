import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { CssBaseline, ThemeProvider } from "@mui/material";
import { App } from "./App";
import { useThemeMode } from "./theme";
import { useGlobalFontScale, useReadingFontFaces } from "./readingSettings";
import { useKeyboardInset } from "./keyboardInset";

function Root(): React.JSX.Element {
  const { theme, mode, setMode } = useThemeMode();
  // Lazy-load + apply the selected reading font (sets --cowboy-reading-font).
  useReadingFontFaces();
  // Apply the font-size scale as a global app text zoom (root <html> font-size).
  useGlobalFontScale();
  // Publish the keyboard overlap as `--kb-inset` so the composer lifts clear of
  // the keyboard + its iOS-native accessory bar.
  useKeyboardInset();
  return (
    <ThemeProvider theme={theme}>
      <CssBaseline />
      <App themeMode={mode} onSetThemeMode={setMode} />
    </ThemeProvider>
  );
}

// iOS keyboard handling lives in CSS now (see index.html): #root is 100dvh and
// the body is locked, with interactive-widget=resizes-content in the viewport
// meta. The browser shrinks the layout viewport — and so #root — when the
// keyboard opens, with no JS. The previous visualViewport --app-height scheme
// is gone: on a real iPhone visualViewport.height is smaller than the
// position:fixed body, so sizing #root from it left a blank strip at the bottom
// (invisible in Chrome's device emulator, which doesn't split visual vs layout
// viewport). Matches the atlantis portal, which uses the same dvh approach.

const el = document.getElementById("root");
if (el) {
  createRoot(el).render(
    <StrictMode>
      <Root />
    </StrictMode>,
  );
}

// macOS native-fullscreen guard (WKWebView / Tauri desktop shell + standalone
// PWA). An Esc keydown the page doesn't consume walks AppKit's responder chain to
// `cancelOperation:`, which EXITS macOS native fullscreen (the green-button
// fullscreen) — jarring when Esc is meant to leave vim insert mode. preventDefault
// (NOT stopPropagation) cancels ONLY that native default; every JS Esc handler
// (vim mode change, MUI modal/sheet close, cancel-turn) still fires because the
// event keeps propagating. Skipped during IME composition so Esc can still cancel
// a pinyin candidate. (Browser Fullscreen-API exit is UA-enforced + unaffected —
// this only cancels the native default action, which, unlike the browser API, IS
// cancelable from the page.)
globalThis.addEventListener(
  "keydown",
  (e: KeyboardEvent): void => {
    if (e.key === "Escape" && !e.isComposing) e.preventDefault();
  },
  { capture: true },
);

// Standalone PWA: register the service worker (offline shell + installable) AND
// keep it fresh. An installed iOS PWA RESUMES its loaded page on reopen — it does
// not re-navigate — so without this it runs whatever bundle it first loaded until
// a manual reload (the recurring "redeploy doesn't show up" trap). So: re-check
// for a new SW whenever the app returns to the foreground (the SW's VERSION bumps
// per web deploy → a new sw.js → install → skipWaiting → activate → claim), and
// reload ONCE when that new worker takes control, picking up the fresh bundle.
if (import.meta.env.PROD && "serviceWorker" in navigator) {
  // Only auto-reload on an UPDATE (a new worker replacing one already in control),
  // never on the first-install claim.
  const hadController = navigator.serviceWorker.controller != null;
  let reloading = false;
  navigator.serviceWorker.addEventListener("controllerchange", () => {
    if (reloading || !hadController) return;
    reloading = true;
    globalThis.location.reload();
  });
  window.addEventListener("load", () => {
    void navigator.serviceWorker.register("/sw.js").then((reg) => {
      const checkForUpdate = (): void => {
        if (globalThis.document.visibilityState === "visible") void reg.update();
      };
      // Phones get backgrounded/foregrounded constantly, so visibilitychange
      // alone refreshes them. A DESKTOP window (incl. the Tauri shell) often
      // stays open + focused for hours — visibilitychange never fires, so it
      // would sit on the old bundle until a manual restart (the reported
      // "desktop auto-update doesn't work"). Poll too: a tiny sw.js fetch every
      // 60s that only does anything when VERSION actually changed.
      globalThis.document.addEventListener("visibilitychange", checkForUpdate);
      globalThis.setInterval(checkForUpdate, 60_000);
      checkForUpdate();
    }).catch(() => {});
  });
}
