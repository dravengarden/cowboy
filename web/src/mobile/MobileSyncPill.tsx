import CheckIcon from "@mui/icons-material/Check";
import CloudOffOutlinedIcon from "@mui/icons-material/CloudOffOutlined";
import ErrorOutlineIcon from "@mui/icons-material/ErrorOutline";
import { alpha, Box, Button, ButtonBase, CircularProgress, Stack, Typography } from "@mui/material";
import { useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { controlPlaneConnection } from "../controlPlane";
import { ObsidianSheet } from "../ObsidianSheet";
import { retrySyncNow, useSyncStatus } from "../store";
import {
  presentedSyncPhase,
  relativeAge,
  SYNC_PRESENTATION_DEBOUNCE_MS,
  type SyncPhase,
  syncStatusDetail,
  syncStatusLabel,
  syncStatusTone,
} from "../syncStatus";

/**
 * The Mobile sync status pill (docs/offline-first-sync.md §User experience).
 * Paint-only chrome under the system clearance: no transform, no shadow, no
 * work on finger-down, so a drawer swipe never pays for it. It appears only
 * after a non-live phase has lasted the presentation debounce, flashes a short
 * "Synced" on recovery, and opens a small sheet with the detail and actions.
 */
export function MobileSyncPill(): ReactNode {
  const status = useSyncStatus();
  const banner = controlPlaneConnection.useConnectionBanner();
  const [now, setNow] = useState(() => Date.now());
  const [open, setOpen] = useState(false);
  const shownRef = useRef<SyncPhase | null>(null);
  const recoveredAtRef = useRef<number | undefined>(undefined);
  const wasLiveRef = useRef(status.phase === "live");

  if (status.phase === "live" && !wasLiveRef.current && shownRef.current !== null) {
    recoveredAtRef.current = Date.now();
  }
  wasLiveRef.current = status.phase === "live";

  const presented = presentedSyncPhase(status, shownRef.current, recoveredAtRef.current, now);
  shownRef.current = presented === null || presented === "recovered" ? null : presented;
  const visible = presented !== null && banner?.kind !== "update";

  // Tick while something is pending: the debounce, a retry countdown, the
  // recovery flash, or the "last synced" age in the open sheet.
  const ticking = status.phase !== "live" || presented !== null || open;
  useEffect(() => {
    if (!ticking) return undefined;
    setNow(Date.now());
    const timer = globalThis.setInterval(() => setNow(Date.now()), 1000);
    return () => globalThis.clearInterval(timer);
  }, [ticking]);
  useEffect(() => {
    if (status.phase === "live") return undefined;
    const timer = globalThis.setTimeout(() => setNow(Date.now()), SYNC_PRESENTATION_DEBOUNCE_MS + 16);
    return () => globalThis.clearTimeout(timer);
  }, [status.phase, status.since]);

  const tone = presented === "recovered" ? "success" : syncStatusTone(status.phase);
  const label = presented === "recovered" ? "Synced" : syncStatusLabel(status, now);
  const busy = status.phase === "connecting" || status.phase === "waiting";
  const synced = relativeAge(status.lastLiveAt, now);

  return (
    <>
      {visible && label !== null && (
        <Box
          role="status"
          aria-live="polite"
          data-mobile-sync-pill={presented}
          sx={{
            position: "absolute",
            top: "calc(var(--cowboy-system-top-clearance, 0px) + 6px)",
            left: 0,
            right: 0,
            display: "flex",
            justifyContent: "center",
            pointerEvents: "none",
            zIndex: (theme) => theme.zIndex.appBar + 1,
          }}
        >
          <ButtonBase
            onClick={() => setOpen(true)}
            disabled={presented === "recovered"}
            sx={{
              pointerEvents: "auto",
              minHeight: 28,
              px: 1.5,
              gap: 0.75,
              borderRadius: 999,
              bgcolor: (theme) => alpha(theme.palette[tone].main, 0.92),
              color: `${tone}.contrastText`,
              fontSize: "0.8125rem",
              fontWeight: 600,
              letterSpacing: "0.01em",
              whiteSpace: "nowrap",
              border: (theme) => `1px solid ${alpha(theme.palette[tone].contrastText, 0.18)}`,
            }}
          >
            {presented === "recovered"
              ? <CheckIcon sx={{ fontSize: "1rem" }} />
              : busy
              ? <CircularProgress size={12} color="inherit" thickness={5} />
              : status.phase === "live"
              ? <ErrorOutlineIcon sx={{ fontSize: "1rem" }} />
              : <CloudOffOutlinedIcon sx={{ fontSize: "1rem" }} />}
            <span>{label}</span>
          </ButtonBase>
        </Box>
      )}
      <ObsidianSheet
        open={open}
        onClose={() => setOpen(false)}
        title="Connection"
        ariaLabel="Connection status"
        actions={
          <Stack direction="row" spacing={1} sx={{ width: "100%" }}>
            <Button
              fullWidth
              variant="outlined"
              onClick={() => setOpen(false)}
              sx={{ borderRadius: 999, textTransform: "none", fontWeight: 600 }}
            >
              Close
            </Button>
            {status.phase === "fenced"
              ? (
                <Button
                  fullWidth
                  variant="contained"
                  onClick={() => globalThis.location.reload()}
                  sx={{ borderRadius: 999, textTransform: "none", fontWeight: 700 }}
                >
                  Reload
                </Button>
              )
              : status.phase !== "live"
              ? (
                <Button
                  fullWidth
                  variant="contained"
                  disabled={status.phase === "auth_required"}
                  onClick={() => {
                    retrySyncNow();
                    setOpen(false);
                  }}
                  sx={{ borderRadius: 999, textTransform: "none", fontWeight: 700 }}
                >
                  Retry now
                </Button>
              )
              : null}
          </Stack>
        }
      >
        <Stack spacing={1.25} sx={{ px: 0.5, pb: 0.5 }}>
          <Typography sx={{ fontWeight: 700, fontSize: "1rem" }}>
            {syncStatusLabel(status, now) ?? "Connected"}
          </Typography>
          <Typography sx={{ color: "text.secondary", fontSize: "0.9rem", lineHeight: 1.45 }}>
            {syncStatusDetail(status, now)}
          </Typography>
          <Stack spacing={0.25} sx={{ color: "text.secondary", fontSize: "0.8125rem" }}>
            {synced !== null && status.phase !== "live" && <span>Last synced {synced}</span>}
            {status.outbox.pending > 0 && (
              <span>
                {status.outbox.pending} queued · sends automatically when Cowboy is reachable
              </span>
            )}
            {status.outbox.held > 0 && (
              <span>{status.outbox.held} waiting for you to retry, edit, or discard</span>
            )}
          </Stack>
        </Stack>
      </ObsidianSheet>
    </>
  );
}
