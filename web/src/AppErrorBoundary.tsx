// AppErrorBoundary — the LAST line of defense against a blank white screen.
//
// React unmounts the ENTIRE tree if any component throws during render with no
// boundary above it — which renders as a blank white page, indistinguishable
// from "the backend is down". cowboy had no top-level boundary (only the
// per-tool `ToolBoundary` in tools/registry.tsx), so a single render bug
// anywhere outside a tool card whited out the whole app. This catches that and
// degrades to a red error card with a reload, so a render crash looks like a
// crash (and stays recoverable) instead of a mystery white screen.
//
// Scope note: an ErrorBoundary only catches errors thrown during React's
// render/commit. It does NOT catch async/event-handler errors or a dead backend
// — those are handled by the WS layer's reconnect + the red ConnectionBanner.
// The two together cover the white-screen surface: this for render crashes, the
// banner for connectivity.

import { Box, Button, Typography } from "@mui/material";
import { Component, type ErrorInfo, type ReactNode } from "react";
import {
  forcedBundleRecoveryUrl,
  isModuleLoadError,
  latestBundleRecoveryUrl,
} from "./moduleRecovery";
import {
  CRASH_INCIDENT_SEVERITY,
  markClientReloadIntent,
  reportClientIncident,
  reportClientLog,
} from "./observability";

interface State {
  readonly error: Error | undefined;
  readonly recovering: boolean;
  readonly recoveryFailed: boolean;
}

const MODULE_RECOVERY_KEY = "cowboy.module-recovery-at";
const MODULE_RECOVERY_COOLDOWN_MS = 30_000;

async function boundedRecoveryFetch(
  input: RequestInfo | URL,
  init?: RequestInit,
): Promise<Response> {
  const controller = new AbortController();
  const timeout = globalThis.setTimeout(() => controller.abort(), 4_000);
  try {
    return await globalThis.fetch(input, { ...init, signal: controller.signal });
  } finally {
    globalThis.clearTimeout(timeout);
  }
}

async function recoverLatestBundle(force = false): Promise<boolean> {
  const now = Date.now();
  // A user-requested retry must actually navigate. The readiness probe below
  // is deliberately conservative for automatic recovery, but it can time out
  // in a stale/backgrounded WKWebView even when a fresh top-level load works.
  // Re-running that probe made the Retry button a no-op while killing and
  // reopening the app recovered immediately.
  if (force) {
    const target = forcedBundleRecoveryUrl(globalThis.location.href, () => now);
    markClientReloadIntent("module_error_manual_retry");
    globalThis.location.replace(target);
    return true;
  }
  const desktopRecovery = (globalThis as typeof globalThis & {
    __cowboyRecoverLatestBundle?: (force?: boolean) => Promise<void>;
  }).__cowboyRecoverLatestBundle;
  if (desktopRecovery) {
    await desktopRecovery(force);
    return true;
  }
  let previous = 0;
  try {
    previous = Number(globalThis.sessionStorage.getItem(MODULE_RECOVERY_KEY) ?? 0);
  } catch {
    // Some embedded/private contexts deny sessionStorage. Recovery must still
    // work; the in-memory page is about to be replaced anyway.
  }
  if (!force && Number.isFinite(previous) && now - previous < MODULE_RECOVERY_COOLDOWN_MS) {
    return false;
  }
  try {
    globalThis.sessionStorage.setItem(MODULE_RECOVERY_KEY, String(now));
  } catch {
    // Best-effort loop guard; continue to the network recovery path.
  }
  // Do not wait for the Service Worker updater: native WKWebView has no SW API,
  // and WebKit can leave registration.update() pending while backgrounded.
  if ("serviceWorker" in navigator) {
    void navigator.serviceWorker.getRegistration()
      .then((registration) => registration?.update())
      .catch(() => undefined);
  }
  const target = await latestBundleRecoveryUrl(
    globalThis.location.href,
    globalThis.location.origin,
    boundedRecoveryFetch,
    () => now,
  );
  if (!target) {
    reportClientLog(
      "warn",
      "module_recovery_probe_failed",
      "Automatic module recovery did not find a ready bundle",
    );
    return false;
  }
  markClientReloadIntent("module_error_automatic_recovery");
  globalThis.location.replace(target);
  return true;
}

export class AppErrorBoundary extends Component<{ children: ReactNode }, State> {
  override state: State = {
    error: undefined,
    recovering: false,
    recoveryFailed: false,
  };

  static getDerivedStateFromError(error: Error): State {
    return { error, recovering: false, recoveryFailed: false };
  }

  private readonly recover = async (force: boolean): Promise<void> => {
    if (this.state.recovering) return;
    this.setState({ recovering: true, recoveryFailed: false });
    const navigating = await recoverLatestBundle(force);
    if (!navigating) {
      this.setState({ recovering: false, recoveryFailed: true });
    }
  };

  override componentDidCatch(error: Error, info: ErrorInfo): void {
    // Keep the stack in the console for a dev/remote-debug session; the card
    // below only shows the message (a full stack is noise for the user).
    // eslint-disable-next-line no-console
    console.error("AppErrorBoundary caught a render error", error, info.componentStack);
    const componentStack = info.componentStack?.slice(0, 512) ?? "";
    reportClientLog("error", "react_render_error", error, {
      component_stack: componentStack,
    });
    reportClientIncident("client_render_failure", CRASH_INCIDENT_SEVERITY, error, {
      component_stack: componentStack,
    });
    if (isModuleLoadError(error)) void this.recover(false);
  }

  override render(): ReactNode {
    const { error, recovering, recoveryFailed } = this.state;
    if (!error) {
      return this.props.children;
    }
    return (
      <Box
        role="alert"
        sx={{
          position: "fixed",
          inset: 0,
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
          justifyContent: "center",
          gap: 2,
          p: 3,
          textAlign: "center",
          // Same red as the ConnectionBanner's "down" state, so a render crash
          // reads as the same severity as a lost connection.
          bgcolor: "error.main",
          color: "error.contrastText",
          // Owns the notch (it's a full-screen takeover).
          pt: "calc(var(--cowboy-system-top-clearance) + 24px)",
        }}
      >
        <Typography variant="h6" fontWeight={600}>
          Something went wrong
        </Typography>
        <Typography variant="body2" sx={{ maxWidth: 480, opacity: 0.9, wordBreak: "break-word" }}>
          {error.message || "The app hit an unexpected error and couldn’t render."}
        </Typography>
        {recoveryFailed && (
          <Typography variant="body2" sx={{ maxWidth: 480, opacity: 0.9 }}>
            The latest app bundle is not ready yet. Check the connection and retry.
          </Typography>
        )}
        <Button
          variant="contained"
          color="inherit"
          disabled={recovering}
          onClick={() => void this.recover(true)}
          sx={{ mt: 1, color: "error.main", fontWeight: 600 }}
        >
          {recovering ? "Checking update…" : recoveryFailed ? "Retry update" : "Update & reload"}
        </Button>
      </Box>
    );
  }
}
