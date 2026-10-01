import { Box, Button } from "@mui/material";
import { useEffect, useState } from "react";
import type { ReactNode } from "react";
import {
  retrySessionHydration,
  useStoreSelector,
  useSyncStatus,
} from "./store";
import { relativeAge } from "./syncStatus";

/**
 * One quiet line under the newest row while a transcript is painted from the
 * local replica and the Hub has not confirmed it yet
 * (docs/offline-first-sync.md §Transcript). Disappears the moment the live
 * snapshot lands; never blocks reading or typing.
 */
export function TranscriptCachedCaption(
  { sessionId }: { readonly sessionId: string },
): ReactNode {
  const source = useStoreSelector((snapshot) =>
    snapshot.transcriptSources.get(sessionId)
  );
  const sync = useSyncStatus();
  const cached = source?.source === "replica";
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!cached) return undefined;
    const timer = globalThis.setInterval(() => setNow(Date.now()), 30_000);
    return () => globalThis.clearInterval(timer);
  }, [cached]);
  if (!cached || source === undefined) return null;
  const age = relativeAge(source.syncedAt, now);
  const verb = sync.phase !== "live" && sync.phase !== "connecting"
    ? "offline"
    : source.syncState === "failed"
    ? "sync failed"
    : source.syncState === "retrying"
    ? "retrying…"
    : "syncing…";
  return (
    <Box
      data-transcript-cached-caption
      sx={{
        alignSelf: "center",
        px: 1.25,
        py: 0.25,
        my: 0.5,
        borderRadius: 999,
        fontSize: "0.75rem",
        color: "text.secondary",
        bgcolor: "action.hover",
        userSelect: "none",
      }}
    >
      {age === null ? `Cached · ${verb}` : `Cached · updated ${age} · ${verb}`}
      {(source.syncState === "failed" || source.syncState === "retrying") && (
        <Button
          size="small"
          onClick={(): void => {
            void retrySessionHydration(sessionId);
          }}
          sx={{ minHeight: 44 }}
        >
          Retry sync
        </Button>
      )}
    </Box>
  );
}
