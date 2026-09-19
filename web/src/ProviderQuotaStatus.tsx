import { useCallback, useEffect, useMemo, useState } from "react";
import { alpha, Box, Stack, Typography } from "@mui/material";
import WarningAmberRounded from "@mui/icons-material/WarningAmberRounded";
import RefreshRounded from "@mui/icons-material/RefreshRounded";
import { NetworkIconButton } from "./NetworkActionFeedback";
import { providerName } from "./providerPresentation";
import { useProviderCatalog } from "./providerCatalog";
import type { Status } from "./protocol";
import {
  exhaustedAccountUsageLimits,
  providerUsage,
  shortResetTime,
  type UsageLimit,
  type UsageSnapshot,
} from "./usageLimits";
import { readUsage, refreshSessionUsage } from "./usageApi";

const USAGE_POLL_MS = 60_000;

function limitDetail(limit: UsageLimit): string {
  return limit.resetsAt === undefined
    ? `${limit.label} limit`
    : `${limit.label} resets ${shortResetTime(limit.resetsAt)}`;
}

/**
 * Persistent, session-local explanation for the otherwise silent native retry
 * that follows an exhausted subscription window. The surface is paint-only:
 * the Mobile composer already rides the workspace swipe compositor, so this
 * must not add its own transform, filter, or shadow layer.
 */
export function ProviderQuotaStatus({
  provider,
  providerVersion,
  providerDigest,
  status,
}: {
  provider: string;
  providerVersion?: string;
  providerDigest?: string;
  status: Status;
}): React.JSX.Element | null {
  const { catalog } = useProviderCatalog();
  const [snapshot, setSnapshot] = useState<UsageSnapshot | null>(null);
  const [clock, setClock] = useState(() => Date.now());

  useEffect(() => {
    const controller = new AbortController();
    const load = (): void => {
      if (document.visibilityState === "hidden") return;
      void readUsage(controller.signal).then((next) => {
        if (!controller.signal.aborted) {
          setSnapshot(next);
          setClock(Date.now());
        }
      }).catch(() => undefined);
    };
    setSnapshot(null);
    load();
    const timer = globalThis.setInterval(load, USAGE_POLL_MS);
    document.addEventListener("visibilitychange", load);
    return (): void => {
      controller.abort();
      globalThis.clearInterval(timer);
      document.removeEventListener("visibilitychange", load);
    };
  }, [provider, providerDigest, providerVersion]);

  useEffect(() => {
    const timer = globalThis.setInterval(() => setClock(Date.now()), 30_000);
    return (): void => globalThis.clearInterval(timer);
  }, []);

  const usage = useMemo(
    () => providerUsage(snapshot, provider, providerVersion, providerDigest),
    [catalog, provider, providerDigest, providerVersion, snapshot],
  );
  const exhausted = useMemo(
    () => exhaustedAccountUsageLimits(usage, clock),
    [clock, usage],
  );
  const refresh = useCallback(async (): Promise<void> => {
    const next = await refreshSessionUsage(
      provider,
      providerVersion,
      providerDigest,
    );
    setSnapshot(next);
    setClock(Date.now());
  }, [provider, providerDigest, providerVersion]);

  if (exhausted.length === 0) return null;
  const activeTurn = status === "busy";
  const details = exhausted.map(limitDetail).join(" · ");
  const displayName = providerName(provider, providerVersion, providerDigest);

  return (
    <Box
      data-provider-quota-status
      data-composer-stack-slot="quota"
      sx={{
        position: "relative",
        display: "flex",
        justifyContent: "center",
        px: 2,
        width: "100%",
        minWidth: 0,
        boxSizing: "border-box",
        zIndex: 3,
      }}
    >
      <Stack
        role="alert"
        aria-live="polite"
        direction="row"
        alignItems="center"
        spacing={1}
        sx={(theme) => ({
          width: "100%",
          maxWidth: 560,
          minWidth: 0,
          px: 1.25,
          py: 1,
          border: 1,
          borderColor: alpha(theme.palette.warning.main, 0.42),
          borderRadius: 2,
          color: "warning.main",
          bgcolor: alpha(
            theme.palette.warning.main,
            theme.palette.mode === "dark" ? 0.16 : 0.09,
          ),
        })}
      >
        <WarningAmberRounded sx={{ fontSize: 20, flexShrink: 0 }} />
        <Box sx={{ flex: 1, minWidth: 0 }}>
          <Typography
            variant="body2"
            sx={{ fontWeight: 700, lineHeight: 1.35 }}
          >
            {displayName} usage limit reached
          </Typography>
          <Typography
            variant="caption"
            color="text.secondary"
            sx={{ display: "block", lineHeight: 1.4 }}
          >
            {details}. {activeTurn
              ? "This turn may wait until the provider makes capacity available."
              : "Sending now may wait until the provider makes capacity available."}
          </Typography>
        </Box>
        <NetworkIconButton
          aria-label="Refresh provider usage"
          size="small"
          reliableTouch
          networkAction={refresh}
          sx={{ flexShrink: 0, color: "warning.main" }}
        >
          <RefreshRounded fontSize="small" />
        </NetworkIconButton>
      </Stack>
    </Box>
  );
}
