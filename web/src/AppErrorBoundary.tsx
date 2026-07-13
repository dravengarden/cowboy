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
import { isModuleLoadError } from "./moduleRecovery";

interface State {
  readonly error: Error | undefined;
}

const MODULE_RECOVERY_KEY = "cowboy.module-recovery-at";
const MODULE_RECOVERY_COOLDOWN_MS = 30_000;

async function recoverLatestBundle(force = false): Promise<void> {
  const now = Date.now();
  let previous = 0;
  try {
    previous = Number(globalThis.sessionStorage.getItem(MODULE_RECOVERY_KEY) ?? 0);
  } catch {
    // Some embedded/private contexts deny sessionStorage. Recovery must still
    // work; the in-memory page is about to be replaced anyway.
  }
  if (!force && Number.isFinite(previous) && now - previous < MODULE_RECOVERY_COOLDOWN_MS) return;
  try {
    globalThis.sessionStorage.setItem(MODULE_RECOVERY_KEY, String(now));
  } catch {
    // Best-effort loop guard; continue to the network recovery path.
  }
  try {
    const registration = await navigator.serviceWorker?.getRegistration();
    await registration?.update();
  } catch {
    // Reload still recovers through the network-first navigation path when the
    // SW update probe itself is unavailable.
  }
  globalThis.location.reload();
}

export class AppErrorBoundary extends Component<{ children: ReactNode }, State> {
  override state: State = { error: undefined };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  override componentDidCatch(error: Error, info: ErrorInfo): void {
    // Keep the stack in the console for a dev/remote-debug session; the card
    // below only shows the message (a full stack is noise for the user).
    // eslint-disable-next-line no-console
    console.error("AppErrorBoundary caught a render error", error, info.componentStack);
    if (isModuleLoadError(error)) void recoverLatestBundle();
  }

  override render(): ReactNode {
    const { error } = this.state;
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
          pt: "calc(env(safe-area-inset-top, 0px) + 24px)",
        }}
      >
        <Typography variant="h6" fontWeight={600}>
          Something went wrong
        </Typography>
        <Typography variant="body2" sx={{ maxWidth: 480, opacity: 0.9, wordBreak: "break-word" }}>
          {error.message || "The app hit an unexpected error and couldn’t render."}
        </Typography>
        <Button
          variant="contained"
          color="inherit"
          onClick={() => void recoverLatestBundle(true)}
          sx={{ mt: 1, color: "error.main", fontWeight: 600 }}
        >
          Update & reload
        </Button>
      </Box>
    );
  }
}
