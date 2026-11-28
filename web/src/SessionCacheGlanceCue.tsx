import { useEffect, useMemo, useState } from "react";
import { alpha, Box, Button, Stack, Tooltip, Typography } from "@mui/material";
import { PREFERENCES_CHANGED } from "./aboutPreferences";
import {
  fetchSessionCacheMetrics,
  type SessionCacheMetricsResponse,
} from "./cacheMetricsApi";
import { useCacheProtectionStatus } from "./ContextRolloverSettings";
import { desktopToolbarActionSx } from "./desktop/desktopToolbarAction";
import type { MetricTone } from "./InfoMetricVisuals";
import { currentProviderEntry, useProviderCatalog } from "./providerCatalog";
import { deriveCacheGlanceCue } from "./sessionCacheUi";
import { openSessionSettings } from "./sessionSettingsOpen";
import { useStoreSelector } from "./store";
import { useLatestTurnEndSeq } from "./turnEndWatermark";

function glanceDotColor(tone: MetricTone): string {
  return tone === "neutral" ? "text.disabled" : `${tone}.main`;
}

/** Calm Layer-1 Prompt Cache glance button for Desktop Composer toolbar.
 *  Neutral in normal warm state; highlights only on expiry, active paid keepalive,
 *  or host-policy cold-resume advisory. Click jumps straight to Run Configuration -> Context. */
export function DesktopCacheGlanceButton({
  sessionId,
}: {
  sessionId: string;
}): React.JSX.Element | null {
  const [data, setData] = useState<SessionCacheMetricsResponse | null>(null);
  const [refreshTick, setRefreshTick] = useState(0);
  const liveWatermark = useLatestTurnEndSeq(sessionId);
  const { catalog } = useProviderCatalog();
  const sessionMeta = useStoreSelector((snapshot) => snapshot.sessionsById.get(sessionId));
  const supportsKeepalive = useMemo(() => {
    if (!sessionMeta || !catalog) return false;
    const entry = currentProviderEntry(
      sessionMeta.provider,
      sessionMeta.provider_version,
      sessionMeta.provider_generation_digest,
    );
    return entry?.manifest.host.features.includes("cache_protection_v1") === true;
  }, [catalog, sessionMeta]);
  const { cacheProtection } = useCacheProtectionStatus(supportsKeepalive, sessionId);

  useEffect(() => {
    const onPrefs = (event: Event): void => {
      const scopes = (event as CustomEvent<Record<string, number> | undefined>).detail;
      if (scopes && !scopes.context_rollover) return;
      setRefreshTick((t) => t + 1);
    };
    const onVisible = (): void => {
      if (document.visibilityState === "visible") setRefreshTick((t) => t + 1);
    };
    window.addEventListener(PREFERENCES_CHANGED, onPrefs);
    document.addEventListener("visibilitychange", onVisible);
    const interval = globalThis.setInterval(onVisible, 60_000);
    return (): void => {
      window.removeEventListener(PREFERENCES_CHANGED, onPrefs);
      document.removeEventListener("visibilitychange", onVisible);
      globalThis.clearInterval(interval);
    };
  }, []);

  useEffect(() => {
    const controller = new AbortController();
    void fetchSessionCacheMetrics(sessionId, 5, controller.signal)
      .then((res) => {
        if (!controller.signal.aborted) setData(res);
      })
      .catch(() => {
        if (!controller.signal.aborted) setData(null);
      });
    return (): void => controller.abort();
  }, [sessionId, liveWatermark, refreshTick, sessionMeta?.status, sessionMeta?.context_used]);

  const current = data && data.session_id === sessionId ? data : null;
  const cue = useMemo(
    () => deriveCacheGlanceCue(current, supportsKeepalive ? cacheProtection : null),
    [current, supportsKeepalive, cacheProtection],
  );
  if (!cue) return null;

  const tooltipContent = (
    <Stack spacing={0.25}>
      {cue.tooltipLines.map((line) => (
        <Typography key={line} variant="caption" sx={{ lineHeight: 1.35 }}>
          {line}
        </Typography>
      ))}
    </Stack>
  );

  const attentionTone = cue.showAttention && cue.tone !== "neutral" ? cue.tone : null;

  return (
    <Tooltip title={tooltipContent}>
      <Button
        size="small"
        color="inherit"
        data-composer-action="cache-glance"
        data-cache-glance-attention={attentionTone ?? undefined}
        aria-label={`Prompt cache: ${cue.shortLabel}`}
        onPointerDown={(event): void => {
          if (event.button === 0) event.preventDefault();
        }}
        onClick={(): void => openSessionSettings("cache", sessionId)}
        sx={[
          desktopToolbarActionSx,
          (theme) =>
            attentionTone !== null
              ? {
                  color: `${attentionTone}.main`,
                  bgcolor: alpha(theme.palette[attentionTone].main, 0.08),
                  "&:hover": { bgcolor: alpha(theme.palette[attentionTone].main, 0.14) },
                }
              : { color: "text.secondary" },
        ]}
      >
        <Box
          aria-hidden="true"
          sx={{
            width: 6,
            height: 6,
            borderRadius: "50%",
            bgcolor: glanceDotColor(cue.tone),
            flexShrink: 0,
          }}
        />
        <Box component="span" sx={{ fontSize: "0.75rem", whiteSpace: "nowrap" }}>
          {cue.shortLabel}
        </Box>
      </Button>
    </Tooltip>
  );
}
