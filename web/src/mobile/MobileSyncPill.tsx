import CheckIcon from "@mui/icons-material/Check";
import CloudOffOutlinedIcon from "@mui/icons-material/CloudOffOutlined";
import { alpha, Box, Button, ButtonBase, CircularProgress, Stack, Typography } from "@mui/material";
import { useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { controlPlaneConnection, useActiveSessionId } from "../controlPlane";
import { ObsidianSheet } from "../ObsidianSheet";
import { requestPickSession } from "../sessionPickRequest";
import { retrySyncNow, useHeldDeliveries, useStoreSelector, useSyncStatus } from "../store";
import { SYNC_SHEET_EVENT } from "../syncSheetRequest";
import {
  presentedSyncPhase,
  relativeAge,
  SYNC_PRESENTATION_DEBOUNCE_MS,
  type SyncPhase,
  syncStatusDetail,
  syncStatusLabel,
  syncStatusTone,
  withHeld,
} from "../syncStatus";
import { useSessionsDrawerOpen } from "./sessionsDrawerPresence";

const pillButtonSx = {
  pointerEvents: "auto",
  minHeight: 44,
  px: 2,
  gap: 0.75,
  borderRadius: 999,
  fontSize: "0.8125rem",
  fontWeight: 600,
  letterSpacing: "0.01em",
  whiteSpace: "nowrap",
} as const;

const sheetButtonSx = { borderRadius: 999, textTransform: "none", fontWeight: 600 } as const;

/**
 * The one Mobile connection indicator (docs/offline-first-sync.md §Mobile).
 * A tinted pill under the system clearance in the same language as the
 * transcript's activity pills: paint-only, no transform, no shadow, no work on
 * finger-down. It appears after a non-live phase has lasted the presentation
 * debounce, flashes "Synced" on recovery, hides while the sessions drawer
 * shows its own status line, and opens one sheet with the detail, the
 * sessions that hold unsent rows, and the actions that fit the state.
 */
export function MobileSyncPill(): ReactNode {
  const raw = useSyncStatus();
  const held = useHeldDeliveries();
  const activeId = useActiveSessionId();
  const sessions = useStoreSelector((snapshot) => snapshot.sessions);
  const drawerOpen = useSessionsDrawerOpen();
  const banner = controlPlaneConnection.useConnectionBanner();
  const [now, setNow] = useState(() => Date.now());
  const [open, setOpen] = useState(false);
  const shownRef = useRef<SyncPhase | null>(null);
  const recoveredAtRef = useRef<number | undefined>(undefined);
  const wasLiveRef = useRef(raw.phase === "live");

  // Earlier sends are resolved at the next send, rather than raising a floating reminder.
  const status = withHeld(raw, 0);

  if (status.phase === "live" && !wasLiveRef.current && shownRef.current !== null) {
    recoveredAtRef.current = Date.now();
  }
  wasLiveRef.current = status.phase === "live";

  const presented = presentedSyncPhase(status, shownRef.current, recoveredAtRef.current, now);
  shownRef.current = presented === null || presented === "recovered" ? null : presented;
  const visible = presented !== null && banner?.kind !== "update" && !drawerOpen;

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
  // Other surfaces (the sessions drawer status line) open the same sheet.
  useEffect(() => {
    const onRequest = (): void => setOpen(true);
    globalThis.addEventListener(SYNC_SHEET_EVENT, onRequest);
    return () => globalThis.removeEventListener(SYNC_SHEET_EVENT, onRequest);
  }, []);

  const tone = presented === "recovered"
    ? "success"
    : syncStatusTone(status.phase);
  const label = presented === "recovered" ? "Synced" : syncStatusLabel(status, now);
  const busy = status.phase === "connecting" || status.phase === "waiting";
  const synced = relativeAge(status.lastLiveAt, now);
  const heldRows = held.sessions.map((entry) => ({
    id: entry.id,
    count: entry.ids.length,
    title: sessions.find((session) => session.id === entry.id)?.title ?? "Untitled session",
    active: entry.id === activeId,
  }));

  const openHeldSession = (id: string): void => {
    setOpen(false);
    requestPickSession(id);
  };

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
            sx={(theme) => ({
              ...pillButtonSx,
              color: `${tone}.main`,
              // The transcript's activity pills are tinted glass; this one is
              // opaque with the same tint so it never blurs moving content.
              bgcolor: alpha(theme.palette.background.paper, 0.96),
              backgroundImage: `linear-gradient(0deg, ${alpha(theme.palette[tone].main, 0.16)}, ${
                alpha(theme.palette[tone].main, 0.16)
              })`,
              border: `1px solid ${alpha(theme.palette[tone].main, 0.28)}`,
            })}
          >
            {presented === "recovered"
              ? <CheckIcon sx={{ fontSize: "1rem" }} />
              : busy
              ? <CircularProgress size={12} color="inherit" thickness={5} />
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
            <Button fullWidth variant="outlined" onClick={() => setOpen(false)} sx={sheetButtonSx}>
              Close
            </Button>
            {status.phase === "fenced"
              ? (
                <Button
                  fullWidth
                  variant="contained"
                  onClick={() => globalThis.location.reload()}
                  sx={{ ...sheetButtonSx, fontWeight: 700 }}
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
                  sx={{ ...sheetButtonSx, fontWeight: 700 }}
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
          </Stack>
          {heldRows.length > 0 && (
            <Stack spacing={0.75}>
              {heldRows.map((row) => (
                <Stack
                  key={row.id}
                  direction="row"
                  alignItems="center"
                  spacing={1}
                  sx={{
                    px: 1.25,
                    py: 0.75,
                    borderRadius: 2,
                    bgcolor: (theme) => alpha(theme.palette.warning.main, 0.08),
                  }}
                >
                  <Stack sx={{ minWidth: 0, flex: 1 }}>
                    <Typography noWrap sx={{ fontWeight: 600, fontSize: "0.9rem" }}>
                      {row.title}
                    </Typography>
                    <Typography sx={{ color: "text.secondary", fontSize: "0.8125rem" }}>
                      {row.count === 1
                        ? "1 message to retry, edit, or discard"
                        : `${String(row.count)} messages to retry, edit, or discard`}
                      {row.active ? " · this session" : ""}
                    </Typography>
                  </Stack>
                  {!row.active && (
                    <Button
                      size="small"
                      variant="outlined"
                      onClick={() => openHeldSession(row.id)}
                      sx={{ ...sheetButtonSx, flexShrink: 0 }}
                    >
                      Open
                    </Button>
                  )}
                </Stack>
              ))}
            </Stack>
          )}
        </Stack>
      </ObsidianSheet>
    </>
  );
}
