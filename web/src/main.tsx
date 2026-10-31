import { lazy, StrictMode, Suspense, useEffect, useSyncExternalStore } from "react";
import { createRoot } from "react-dom/client";
import { CssBaseline, ThemeProvider } from "@mui/material";
import { AppErrorBoundary } from "./AppErrorBoundary";
import { ProductAuthGate } from "./auth/ProductAuthGate";
import {
  captureDeviceAuthorizationFromLocation,
  DeviceAuthorizationRoute,
} from "./auth/DeviceAuthorizationPage";
import { BootSkeleton } from "./BootSkeleton";
import { rememberBootSurface } from "./bootSnapshot";
import { SurfaceProvider, useSurfaceProfile } from "./surface/SurfaceProfile";
import { useThemeMode } from "./theme";
import { useGlobalFontScale, useReadingFontFaces } from "./readingSettings";
import { useKeyboardInset } from "./keyboardInset";
import { useSheetFocusReveal } from "./sheetFocusReveal";
import { installHaptics } from "@cowboy/app-shell";
import {
  installObservability,
  markClientReloadIntent,
  reportClientLog,
} from "./observability";
import { createServiceWorkerUpdateCheck } from "./serviceWorkerUpdates";
import { conn } from "./clientConnection";
import { isBundleRecoveryUrl } from "./moduleRecovery";
import {
  clearUpdateAttempt,
  rollbackToPreviousBuild,
  rolledBackNavigationUrl,
  type UpdateAttempt,
} from "./updateAttempt";
import { initializeAppIcons } from "./appIcons";
import { ownPluginHostLifecycle } from "./pluginHost/lifecycle";
import { installBrowserDevice } from "./browserDevice";
import { DocumentNotifications } from "./documents/DocumentNotifications";
import { UndoHistoryHost } from "./UndoHistorySheet";
import { EditorExtensionsDialog } from "./editorExtensions/EditorExtensionsDialog";
import { lowPriorityInterval } from "./network/bandwidth";
import { restoreSubscriptionSignIn, subscribeSignInLocation, subscriptionSignInLocation } from "./subscriptionSignInLink";

restoreSubscriptionSignIn();
installBrowserDevice();

// How long a freshly swapped-in build has to stay up before it counts as
// started. Long enough to cover the gates, the lazy surface chunk and its
// first render; short enough that a user who kills a white screen has not
// already outlasted it.
const UPDATE_SETTLED_MS = 8_000;

const releasePluginHostScope = ownPluginHostLifecycle(globalThis);
import.meta.hot?.dispose(releasePluginHostScope);

initializeAppIcons();

const DesktopApp = lazy(async () => {
  const module = await import("./desktop/DesktopApp");
  return { default: module.DesktopApp };
});
const SubscriptionSignInHandoff = lazy(async () => {
  const module = await import("./SubscriptionSignInHandoff");
  return { default: module.SubscriptionSignInHandoff };
});
const MobileApp = lazy(async () => {
  // WebKit can retain a failed module record across same-window navigations.
  // The recovery URL must therefore select a distinct Vite chunk, not merely
  // cache-bust index.html; closing the installed PWA used to be the only way to
  // clear that failed MobileApp import.
  const module: typeof import("./mobile/MobileApp") = isBundleRecoveryUrl(globalThis.location.href)
    // @ts-expect-error Vite owns query-qualified module identities at build time.
    ? await import("./mobile/MobileApp?bundle-recovery")
    : await import("./mobile/MobileApp");
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
  // Keep a focused sheet field above the keyboard and the sheet footer.
  useSheetFocusReveal();
  const surface = useSurfaceProfile();
  // The static boot shell cannot run this rule, so it reuses the answer
  // (docs/offline-first-sync.md §Boot presentation).
  const desktop = surface.kind === "desktop";
  useEffect(() => rememberBootSurface(desktop), [desktop]);
  // Sign for the swap. Not on mount: a build that mounts and then throws has
  // not started, and the boundary below needs the marker still there to know a
  // crash belongs to the update. Staying up for this long is the evidence.
  useEffect(() => {
    const settled = globalThis.setTimeout(
      () => clearUpdateAttempt(globalThis.localStorage),
      UPDATE_SETTLED_MS,
    );
    return () => globalThis.clearTimeout(settled);
  }, []);
  const deviceAuthorizationActive = captureDeviceAuthorizationFromLocation();
  const subscriptionSignIn = useSyncExternalStore(subscribeSignInLocation, subscriptionSignInLocation, () => "");
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
        <ProductAuthGate>
          <DeviceAuthorizationRoute active={deviceAuthorizationActive}>
            <Suspense fallback={<BootSkeleton />}>{subscriptionSignIn ? <SubscriptionSignInHandoff key={subscriptionSignIn} hash={subscriptionSignIn} /> : app}</Suspense>
            <DocumentNotifications mobile={!desktop} />
            <UndoHistoryHost />
            <EditorExtensionsDialog />
          </DeviceAuthorizationRoute>
        </ProductAuthGate>
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
// popup app-wide (see @cowboy/app-shell/haptic-delegation). Composes with the explicit
// haptic() calls (queue/send, turn-end notifications, long-press, drag) via the
// primitive's coalesce window — no double-buzz. Installed once, never torn down.
installHaptics();
installObservability();

// The document's inline ledger found that the previous load reached this
// point and never came up: the build we swapped into does not start here.
// Put back the one that did before rendering anything — the tree we would
// render is the tree that failed. The service worker keeps two generations,
// so the previous build is whole in cache and this needs no network.
const bootFailure = (globalThis as typeof globalThis & {
  __cowboyUpdateBootFailed?: UpdateAttempt;
}).__cowboyUpdateBootFailed;
if (bootFailure) {
  reportClientLog(
    "error",
    "client_update_boot_failed",
    "A deployed Cowboy build did not start; rolling back",
    { target_build: bootFailure.to },
  );
  void rollbackToPreviousBuild().then((result) => {
    if (!result.ok) {
      // Nothing to fall back to. Running the build we have beats showing
      // nothing, and the next deploy is the way out.
      reportClientLog(
        "warn",
        "client_update_rollback_unavailable",
        "No previous Cowboy build was cached to roll back to",
        { target_build: bootFailure.to },
      );
      return;
    }
    markClientReloadIntent("update_boot_failure_rollback", result.version);
    globalThis.location.replace(
      rolledBackNavigationUrl(globalThis.location.href, Date.now()),
    );
  });
}

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

// Every production surface compares its loaded Vite entry with the deployed
// index on foreground/resume. Native iOS WKWebView does not expose Service
// Workers, so this network probe is the shared authority; browser/PWA surfaces
// additionally ask their registration to update. Detection only raises the
// update state; the shared countdown policy decides when the page may replace
// itself, so a deploy never lands while the user is reading or composing.
if (import.meta.env.PROD) {
  // The cache-first navigation has finished. Do not make a later offline
  // launch network-first merely because this one used a native update URL.
  const updateUrl = new URL(globalThis.location.href);
  if (updateUrl.searchParams.has("cowboy-update")) {
    updateUrl.searchParams.delete("cowboy-update");
    globalThis.history.replaceState(globalThis.history.state, "", updateUrl);
  }
  const installChecks = (updateServiceWorker: () => Promise<unknown>): void => {
    let trigger = "startup";
    const check = createServiceWorkerUpdateCheck(() => {
      // A pending WebKit worker update must not block the bundle authority.
      void updateServiceWorker().catch(() => {});
      return conn.checkForUpdate(trigger);
    });
    const checkForUpdate = (event?: Event): void => {
      // WKWebView can remain `visible` through an app background/resume, so
      // only suppress checks when it explicitly reports `hidden`.
      if (globalThis.document.visibilityState !== "hidden") {
        trigger = event?.type ?? "periodic";
        check();
      }
    };
    globalThis.document.addEventListener("visibilitychange", checkForUpdate);
    // A worker can claim a window already running its build (or be replaced
    // only because an imported script changed). Controller identity is not
    // evidence that the page is stale. Verify the deployed entry on this path,
    // exactly as on focus/resume; an offline check must leave this page up.
    if ("serviceWorker" in navigator) {
      navigator.serviceWorker.addEventListener("controllerchange", checkForUpdate);
    }
    globalThis.addEventListener("pageshow", checkForUpdate);
    globalThis.addEventListener("focus", checkForUpdate);
    globalThis.addEventListener("online", checkForUpdate);
    // The native iOS shell emits this from UIApplication.didBecomeActive. It
    // closes the last lifecycle gap where WKWebView emits no standard event.
    globalThis.addEventListener("cowboy:native-resume", checkForUpdate);
    lowPriorityInterval(checkForUpdate, 60_000);
    checkForUpdate();
  };

  window.addEventListener("load", () => {
    if (!("serviceWorker" in navigator)) {
      installChecks(() => Promise.resolve());
      return;
    }
    void navigator.serviceWorker.register("/sw.js")
      .then((reg) => installChecks(() => reg.update()))
      .catch(() => {
        // Registration failure must not disable the universal bundle probe.
        installChecks(() => Promise.resolve());
      });
  });
}
