import CheckIcon from "@mui/icons-material/Check";
import CloudOffOutlinedIcon from "@mui/icons-material/CloudOffOutlined";
import { alpha, Box, Button, Stack, Typography } from "@mui/material";
import { useEffect, useRef, useState } from "react";
import { retrySyncNow, useSyncStatus } from "../store";
import {
  presentedSyncPhase,
  SYNC_PRESENTATION_DEBOUNCE_MS,
  type SyncPhase,
} from "../syncStatus";
import { connectionNotice } from "./connectionNotice";

/**
 * Strip above the Desktop composer while the Cowboy server cannot be used.
 * It waits out the presentation debounce so a reconnect blip never flashes,
 * says whether the device or the server is the problem, and confirms the
 * recovery briefly before it leaves.
 */
export function DesktopConnectionNotice(): React.JSX.Element | null {
  const status = useSyncStatus();
  const [now, setNow] = useState(() => Date.now());
  const shownRef = useRef<SyncPhase | null>(null);
  const recoveredAtRef = useRef<number | undefined>(undefined);
  const wasLiveRef = useRef(status.phase === "live");

  if (
    status.phase === "live" && !wasLiveRef.current && shownRef.current !== null
  ) {
    recoveredAtRef.current = Date.now();
  }
  wasLiveRef.current = status.phase === "live";
  const presented = presentedSyncPhase(
    status,
    shownRef.current,
    recoveredAtRef.current,
    now,
  );
  shownRef.current = presented === null || presented === "recovered"
    ? null
    : presented;

  // Tick for the debounce, the retry countdown and the recovery flash.
  const ticking = status.phase !== "live" || presented !== null;
  useEffect(() => {
    if (!ticking) return undefined;
    setNow(Date.now());
    const timer = globalThis.setInterval(() => setNow(Date.now()), 1000);
    return () => globalThis.clearInterval(timer);
  }, [ticking]);
  useEffect(() => {
    if (status.phase === "live") return undefined;
    const timer = globalThis.setTimeout(
      () => setNow(Date.now()),
      SYNC_PRESENTATION_DEBOUNCE_MS + 16,
    );
    return () => globalThis.clearTimeout(timer);
  }, [status.phase, status.since]);

  const notice = connectionNotice(status, presented, now);
  if (notice === null) return null;
  const color = `${notice.tone}.main`;
  return (
    <Box
      role="status"
      aria-live="polite"
      data-desktop-connection-notice={notice.tone}
      sx={{
        mx: 1,
        mt: 0.75,
        px: 1.25,
        py: 0.75,
        display: "flex",
        alignItems: "flex-start",
        gap: 1,
        borderRadius: 1.5,
        border: 1,
        borderColor: (theme) => alpha(theme.palette[notice.tone].main, 0.32),
        bgcolor: (theme) => alpha(theme.palette[notice.tone].main, 0.08),
      }}
    >
      {notice.tone === "success"
        ? <CheckIcon sx={{ fontSize: 16, mt: "2px", color }} />
        : <CloudOffOutlinedIcon sx={{ fontSize: 16, mt: "2px", color }} />}
      <Stack spacing={0.25} sx={{ minWidth: 0, flex: 1 }}>
        <Typography
          variant="body2"
          sx={{ fontWeight: 650, color, lineHeight: 1.4 }}
        >
          {notice.title}
        </Typography>
        {notice.hint !== null && (
          <Typography
            variant="caption"
            sx={{ color: "text.primary", lineHeight: 1.4 }}
          >
            {notice.hint}
          </Typography>
        )}
        {notice.meta !== null && (
          <Typography
            variant="caption"
            sx={{ color: "text.secondary", lineHeight: 1.4 }}
          >
            {notice.meta}
          </Typography>
        )}
      </Stack>
      {notice.canRetry && (
        <Button
          size="small"
          variant="outlined"
          color="inherit"
          onClick={(): void => retrySyncNow()}
          sx={{
            flexShrink: 0,
            textTransform: "none",
            fontWeight: 600,
            borderRadius: 999,
            py: 0,
            minHeight: 26,
          }}
        >
          Retry now
        </Button>
      )}
    </Box>
  );
}
