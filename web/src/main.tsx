import { lazy, StrictMode, Suspense, useEffect } from "react";
import { createRoot } from "react-dom/client";
import { CssBaseline, ThemeProvider } from "@mui/material";
import { AppErrorBoundary } from "./AppErrorBoundary";
import { SurfaceProvider, useSurfaceProfile } from "./surface/SurfaceProfile";
import { useThemeMode } from "./theme";
import { useGlobalFontScale, useReadingFontFaces } from "./readingSettings";
import { useKeyboardInset } from "./keyboardInset";
import { installHaptics } from "./_shell";
import { installObservability } from "./observability";
import {
  checkForDeployedUpdate,
  createServiceWorkerUpdateCheck,
} from "./serviceWorkerUpdates";

const DesktopApp = lazy(async () => {
  const module = await import("./desktop/DesktopApp");
  return { default: module.DesktopApp };
});
const MobileApp = lazy(async () => {
  const module = await import("./mobile/MobileApp");
  return { default: module.MobileApp };
});

function Root(): React.JSX.Element {
  const { theme, mode, setMode } = useThemeMode();
  // Lazy-load + apply the selected reading font (sets --cowboy-reading-font).
  useReadingFontFaces();
  // Apply the font-size scale as a global app text zoom (root <html> font-size).
  useGlobalFontScale();
  // Publish the keyboard overlap as `--kb-inset` so the composer lifts clear of
  // the keyboard + its iOS-native accessory bar.
  useKeyboardInset();
  const surface = useSurfaceProfile();
  useEffect(() => {
    if (surface.kind !== "desktop") return undefined;
    const claimModalEscape = (event: KeyboardEvent): void => {
      if (
        event.key === "Escape" && !event.isComposing &&
        document.querySelector(".MuiModal-root") !== null
      ) {
        // MUI Modal stops Escape propagation after running its close handler.
        // Claim the native default in capture without stopping propagation, so
        // the modal still closes but AppKit never interprets the same key as
        // "leave native fullscreen". Outside a modal the later bubble guard
        // remains authoritative, preserving CodeMirror/Vim Escape handling.
        event.preventDefault();
      }
    };
    globalThis.addEventListener("keydown", claimModalEscape, true);
    return (): void => globalThis.removeEventListener("keydown", claimModalEscape, true);
  }, [surface.kind]);
  const app = surface.kind === "desktop"
    ? <DesktopApp themeMode={mode} onSetThemeMode={setMode} />
    : <MobileApp themeMode={mode} onSetThemeMode={setMode} />;
  return (
    <ThemeProvider theme={theme}>
      {/* `enableColorScheme` writes `:root { color-scheme: light|dark }` from the
          ACTIVE theme's `palette.mode`. Without it the iOS keyboard (and UA form
          controls / scrollbars) followed the DEVICE system appearance, so a
          light-themed app on a dark-mode phone got a dark keyboard — mismatched.
          Now the keyboard tracks the in-app theme and flips live when it does. */}
      <CssBaseline enableColorScheme />
      {/* Top-level boundary: a render crash anywhere in <App> degrades to a red
          error card with a reload instead of a blank white screen. */}
      <AppErrorBoundary>
        <Suspense fallback={null}>{app}</Suspense>
      </AppErrorBoundary>
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

// Global MUI haptic delegation: one listener set buzzes every button / switch /
// popup app-wide (see _shell/haptic-delegation). Composes with the explicit
// haptic() calls (queue/send, turn-end notifications, long-press, drag) via the
// primitive's coalesce window — no double-buzz. Installed once, never torn down.
installHaptics();
installObservability();

const el = document.getElementById("root");
if (el) {
  createRoot(el).render(
    <StrictMode>
      <SurfaceProvider>
        <Root />
      </SurfaceProvider>
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

// Every production surface compares its loaded Vite entry with the deployed
// index on foreground/resume. Native iOS WKWebView does not expose Service
// Workers, so this network probe is the shared authority; browser/PWA surfaces
// additionally ask their registration to update. Desktop owns its short
// countdown and Mobile requires an explicit Update tap, so detection never
// replaces a page while the user is reading or composing.
if (import.meta.env.PROD) {
  const loadedEntry = globalThis.document.querySelector<HTMLScriptElement>(
    'script[type="module"][src]',
  )?.getAttribute("src") ?? undefined;
  const reportUpdate = async (): Promise<void> => {
    const { conn } = await import("./store");
    conn.updateAvailable();
  };
  const installChecks = (updateServiceWorker: () => Promise<unknown>): void => {
    const check = createServiceWorkerUpdateCheck(() =>
      checkForDeployedUpdate(
        loadedEntry,
        updateServiceWorker,
        () =>
          globalThis.fetch(
            `/?cowboy-bundle-probe=${Date.now()}`,
            { cache: "no-store" },
          ),
        reportUpdate,
      )
    );
    const checkForUpdate = (): void => {
      // WKWebView can remain `visible` through an app background/resume, so
      // only suppress checks when it explicitly reports `hidden`.
      if (globalThis.document.visibilityState !== "hidden") check();
    };
    globalThis.document.addEventListener("visibilitychange", checkForUpdate);
    globalThis.addEventListener("pageshow", checkForUpdate);
    globalThis.addEventListener("focus", checkForUpdate);
    globalThis.addEventListener("online", checkForUpdate);
    // The native iOS shell emits this from UIApplication.didBecomeActive. It
    // closes the last lifecycle gap where WKWebView emits no standard event.
    globalThis.addEventListener("cowboy:native-resume", checkForUpdate);
    globalThis.setInterval(checkForUpdate, 60_000);
    checkForUpdate();
  };

  window.addEventListener("load", () => {
    if (!("serviceWorker" in navigator)) {
      installChecks(() => Promise.resolve());
      return;
    }
    // Only surface controller changes for an update, never a first install.
    const hadController = navigator.serviceWorker.controller != null;
    let reportedControllerUpdate = false;
    navigator.serviceWorker.addEventListener("controllerchange", () => {
      if (reportedControllerUpdate || !hadController) return;
      reportedControllerUpdate = true;
      void reportUpdate().catch(() => {
        // If the old module graph is already unavailable, reload is the only
        // recovery path. Normal updates take the visible update route above.
        globalThis.location.reload();
      });
    });
    void navigator.serviceWorker.register("/sw.js")
      .then((reg) => installChecks(() => reg.update()))
      .catch(() => {
        // Registration failure must not disable the universal bundle probe.
        installChecks(() => Promise.resolve());
      });
  });
}
