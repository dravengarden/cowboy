import CloudOffOutlined from "@mui/icons-material/CloudOffOutlined";
import CloudUploadOutlined from "@mui/icons-material/CloudUploadOutlined";
import ErrorOutline from "@mui/icons-material/ErrorOutline";
import { Box, Tooltip, Typography } from "@mui/material";
import { useEffect, useState } from "react";
import { useCachedTailSessions, useSessionObligations, useSyncStatus } from "./store";
import { relativeAge, SYNC_PRESENTATION_DEBOUNCE_MS } from "./syncStatus";

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

/** "Last synced …" line for the sessions drawer while the Hub is not live. It
 * waits out the presentation debounce so a reconnect blip never flashes it. */
export function SessionsSyncedCaption(): React.JSX.Element | null {
  const status = useSyncStatus();
  const [now, setNow] = useState(() => Date.now());
  const live = status.phase === "live";
  useEffect(() => {
    if (live) return;
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 15_000);
    return () => clearInterval(timer);
  }, [live, status.since]);
  if (live || now - status.since < SYNC_PRESENTATION_DEBOUNCE_MS) return null;
  const age = relativeAge(status.lastLiveAt, now);
  return (
    <Typography
      variant="caption"
      role="status"
      sx={{ display: "block", px: 2, py: 0.5, color: "text.secondary" }}
    >
      {age === null ? "Not synced on this device yet" : `Last synced ${age}`}
    </Typography>
  );
}
