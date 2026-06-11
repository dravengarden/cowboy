import { StrictMode, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { CssBaseline, ThemeProvider } from "@mui/material";
import { App } from "./App";
import { useThemeMode } from "./theme";
import { useGlobalFontScale, useReadingFontFaces } from "./readingSettings";
import { useKeyboardInset } from "./keyboardInset";
import { installHaptics } from "./_shell";

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
      <DebugSelOverlay />
    </ThemeProvider>
  );
}

// TEMP diagnostic bar (remove after the long-press-paste investigation). A thin
// top strip showing the live focus + selection state, so a device screenshot
// during a long-press reveals: is the native-shell flag set, is drawSelection
// drawing the caret, where does the selection land, does it re-sync on release.
function DebugSelOverlay(): React.JSX.Element {
  const [info, setInfo] = useState("dbg…");
  useEffect(() => {
    const w = globalThis as unknown as { __cowboyNativeShell?: boolean };
    const update = (): void => {
      const ae = document.activeElement;
      const sel = globalThis.getSelection();
      const an = sel?.anchorNode ?? null;
      const anDesc = an === null
        ? "null"
        : an.nodeType === 3
        ? `#text"${(an.textContent ?? "").slice(0, 6)}"`
        : ((an as Element).className || (an as Element).tagName || "?").toString().slice(0, 18);
      const aeDesc = ae === null
        ? "null"
        : ((ae as Element).className || ae.tagName || "?").toString().slice(0, 18);
      const lines = document.querySelectorAll(".cm-content .cm-line").length;
      setInfo(
        `ns=${String(w.__cowboyNativeShell)} drawn=${
          document.querySelector(".cm-cursor") !== null ? "Y" : "n"
        } lines=${String(lines)} | ae=${aeDesc} | selNode=${anDesc} o=${
          String(sel?.anchorOffset)
        } collapsed=${String(sel?.isCollapsed)}`,
      );
    };
    update();
    document.addEventListener("selectionchange", update);
    const iv = globalThis.setInterval(update, 250);
    return (): void => {
      document.removeEventListener("selectionchange", update);
      globalThis.clearInterval(iv);
    };
  }, []);
  return (
    <div
      style={{
        position: "fixed",
        top: 0,
        left: 0,
        right: 0,
        zIndex: 2147483647,
        background: "rgba(0,0,0,0.82)",
        color: "#3f6",
        font: "9px ui-monospace, monospace",
        padding: "1px 4px",
        pointerEvents: "none",
        whiteSpace: "nowrap",
        overflow: "hidden",
      }}
    >
      {info}
    </div>
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

// Global MUI haptic delegation: one listener set buzzes every button / switch /
// popup app-wide (see _shell/haptic-delegation). Composes with the explicit
// haptic() calls (queue/send, turn-end notifications, long-press, drag) via the
// primitive's coalesce window — no double-buzz. Installed once, never torn down.
installHaptics();

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
// fullscreen) — jarring when Esc is meant to leave vim insert mode. We cancel that
// ONE native default by preventDefault-ing Escape — but in the BUBBLE phase, NOT
// capture: CodeMirror skips a keydown whose `defaultPrevented` is already set (it
// checks the flag), so a capture-phase preventDefault silently broke vim's
// insert→normal. Bubbling means the editor (and every other JS Esc handler — modal
// close, cancel-turn) runs FIRST and unaffected; we only stamp preventDefault
// afterwards to suppress the native exit. Skipped during IME composition so Esc can
// still cancel a pinyin candidate. (Browser Fullscreen-API exit is UA-enforced +
// uncancelable; the native responder default, unlike it, IS cancelable.)
globalThis.addEventListener("keydown", (e: KeyboardEvent): void => {
  if (e.key === "Escape" && !e.isComposing) e.preventDefault();
});

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
