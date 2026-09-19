import CloudOffOutlined from "@mui/icons-material/CloudOffOutlined";
import CloudUploadOutlined from "@mui/icons-material/CloudUploadOutlined";
import ErrorOutline from "@mui/icons-material/ErrorOutline";
import { Box, ButtonBase, CircularProgress, Tooltip, Typography } from "@mui/material";
import { useEffect, useState } from "react";
import { useCachedTailSessions, useSessionObligations, useSyncStatus } from "./store";
import { requestSyncSheet } from "./syncSheetRequest";
import {
  relativeAge,
  SYNC_PRESENTATION_DEBOUNCE_MS,
  syncStatusLabel,
  syncStatusTone,
} from "./syncStatus";

// Session-list chrome for the offline-first contract
// (docs/offline-first-sync.md §User experience, "Sessions drawer"). Every
// element here sits on the Mobile swipe path, so it is paint-only: no
// transform, no shadow, no work on finger-down.

/** How many rows of a session still wait to send, or need the user's decision. */
export function SessionObligationBadge(
  { sessionId }: { sessionId: string },
): React.JSX.Element | null {
  const { pending, held } = useSessionObligations(sessionId);
  const total = pending + held;
  if (total === 0) return null;
  const attention = held > 0;
  const label = attention
    ? `${String(held)} ${held === 1 ? "message needs" : "messages need"} attention`
    : `${String(pending)} ${pending === 1 ? "message waits" : "messages wait"} to send`;
  return (
    <Tooltip title={label} enterDelay={300}>
      <Box
        role="img"
        aria-label={label}
        sx={{
          display: "inline-flex",
          alignItems: "center",
          gap: 0.25,
          flexShrink: 0,
          color: attention ? "warning.main" : "info.main",
        }}
      >
        {attention
          ? <ErrorOutline sx={{ fontSize: 15 }} />
          : <CloudUploadOutlined sx={{ fontSize: 15 }} />}
        <Typography
          variant="caption"
          sx={{ fontVariantNumeric: "tabular-nums", lineHeight: 1 }}
        >
          {total}
        </Typography>
      </Box>
    </Tooltip>
  );
}

/** Marks a session whose transcript this device has not cached while the Hub
 * cannot be reached, so the user knows before tapping it. */
export function SessionCacheGlyph(
  { sessionId, active }: { sessionId: string; active: boolean },
): React.JSX.Element | null {
  const status = useSyncStatus();
  const cached = useCachedTailSessions();
  if (active || status.phase === "live" || cached.has(sessionId)) return null;
  return (
    <Tooltip title="Not cached on this device" enterDelay={300}>
      <CloudOffOutlined
        aria-label="Not cached on this device"
        sx={{ fontSize: 15, flexShrink: 0, color: "text.disabled" }}
      />
    </Tooltip>
  );
}

/** The sessions drawer's own connection line while the Hub is not live: the
 * phase, when the list was last synced, and a tap into the connection sheet.
 * The floating pill hides while the drawer is open, so this is the single
 * indicator on that surface. It waits out the presentation debounce so a
 * reconnect blip never flashes it. */
export function SessionsSyncedCaption(): React.JSX.Element | null {
  const status = useSyncStatus();
  const [now, setNow] = useState(() => Date.now());
  const live = status.phase === "live";
  useEffect(() => {
    if (live) return;
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 1_000);
    return () => clearInterval(timer);
  }, [live, status.since]);
  if (live || now - status.since < SYNC_PRESENTATION_DEBOUNCE_MS) return null;
  const age = relativeAge(status.lastLiveAt, now);
  const label = syncStatusLabel(status, now) ?? "Reconnecting…";
  const tone = syncStatusTone(status.phase);
  const busy = status.phase === "connecting" || status.phase === "waiting";
  return (
    <ButtonBase
      role="status"
      aria-live="polite"
      onClick={requestSyncSheet}
      sx={{
        display: "flex",
        width: "100%",
        justifyContent: "flex-start",
        alignItems: "center",
        gap: 0.75,
        px: 2,
        py: 0.75,
        textAlign: "left",
        color: `${tone}.main`,
      }}
    >
      {busy
        ? <CircularProgress size={12} color="inherit" thickness={5} />
        : <CloudOffOutlined sx={{ fontSize: 15, flexShrink: 0 }} />}
      <Typography variant="caption" sx={{ fontWeight: 600 }}>{label}</Typography>
      <Typography variant="caption" sx={{ color: "text.secondary", minWidth: 0 }} noWrap>
        {age === null ? "· list may be out of date" : `· list from ${age}`}
      </Typography>
    </ButtonBase>
  );
}
