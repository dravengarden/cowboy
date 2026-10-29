import { glyphSize } from "./surface/desktopSize";
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
  shortResetTime,
  providerUsageAccount,
  type UsageLimit,
  usageRefreshing,
} from "./usageLimits";
import { loadUsageSnapshot, requestUsageRefresh, useStoreSelector } from "./store";
import { lowPriorityInterval } from "./network/bandwidth";
import { useSubscriptionAccounts } from "./subscriptionAccounts";
import { sessionAccountIdentity } from "./accountIdentity";
import { sessionAccountQuota } from "./usageAccounts";
import { useNativeAccountUsage } from "./nativeAccountUsage";

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
  sessionId,
  provider,
  providerVersion,
  providerDigest,
  status,
  desktop = false,
}: {
  sessionId: string;
  provider: string;
  providerVersion?: string;
  providerDigest?: string;
  status: Status;
  desktop?: boolean;
}): React.JSX.Element | null {
  const { catalog } = useProviderCatalog();
  // Shared Controller-owned usage: a refresh from any device updates it.
  const snapshot = useStoreSelector((state) => state.usage);
  const sessions = useStoreSelector((state) => state.sessions);
  const session = sessions.find((item) => item.id === sessionId && item.provider === provider);
  const serviceAccount = Boolean(session && (!session.provider_account_id || session.provider_account_id === "default"));
  const { data: accounts } = useSubscriptionAccounts();
  const native = useNativeAccountUsage(accounts?.registry.revision);
  const [clock, setClock] = useState(() => Date.now());

  useEffect(() => {
    if (!serviceAccount) return;
    const controller = new AbortController();
    const load = (): void => {
      if (document.visibilityState === "hidden") return;
      void loadUsageSnapshot(controller.signal).catch(() => undefined);
    };
    load();
    const stop = lowPriorityInterval(load, USAGE_POLL_MS);
    document.addEventListener("visibilitychange", load);
    return (): void => {
      controller.abort();
      stop();
      document.removeEventListener("visibilitychange", load);
    };
  }, [provider, providerDigest, providerVersion, serviceAccount]);
  useEffect(() => setClock(Date.now()), [snapshot]);

  useEffect(() => {
    const timer = globalThis.setInterval(() => setClock(Date.now()), 30_000);
    return (): void => globalThis.clearInterval(timer);
  }, []);

  const usage = useMemo(
    () => sessionAccountQuota(snapshot, accounts, sessions, session, native.accounts),
    [catalog, accounts, session, sessions, snapshot, native.accounts],
  );
  const exhausted = useMemo(
    () => exhaustedAccountUsageLimits(usage, clock),
    [clock, usage],
  );
  const account = serviceAccount ? providerUsageAccount(provider, providerVersion, providerDigest) : undefined;
  const named = native.accounts.find((item) => item.account_id === session?.provider_account_id);
  const refreshing = account !== undefined ? usageRefreshing(snapshot, account) : named?.refreshing ?? false;
  const refresh = useCallback(async (): Promise<void> => {
    if (named?.available) {
      await native.refresh(named.account_id);
      return;
    }
    if (!account) {
      throw new Error("Usage is unavailable for this session's Provider.");
    }
    await requestUsageRefresh(account);
  }, [account, named, native.refresh]);

  if (exhausted.length === 0) return null;
  const activeTurn = status === "busy";
  const details = exhausted.map(limitDetail).join(" · ");
  const identity = session && sessionAccountIdentity(accounts, session);
  const displayName = identity?.label ?? providerName(provider, providerVersion, providerDigest);

  return (
    <Box
      data-provider-quota-status
      data-composer-stack-slot="quota"
      sx={{
        position: "relative",
        display: "flex",
        justifyContent: "center",
        px: desktop ? 0 : 2,
        ...(desktop && { flexShrink: 0 }),
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
          maxWidth: desktop ? "none" : 560,
          minWidth: 0,
          px: 1.25,
          py: desktop ? 0.5 : 1,
          border: desktop ? 0 : 1,
          ...(desktop && { borderBottom: 1 }),
          borderColor: alpha(theme.palette.warning.main, 0.42),
          borderRadius: desktop ? 0 : 2,
          color: "warning.main",
          bgcolor: alpha(
            theme.palette.warning.main,
            theme.palette.mode === "dark" ? 0.16 : 0.09,
          ),
        })}
      >
        <WarningAmberRounded sx={{ fontSize: glyphSize(20), flexShrink: 0 }} />
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
        {(serviceAccount || named?.available) && (
          <NetworkIconButton
            aria-label="Refresh provider usage"
            size="small"
            reliableTouch
            networkAction={refresh}
            disabled={refreshing}
            sx={{ flexShrink: 0, color: "warning.main" }}
          >
            <RefreshRounded fontSize="small" />
          </NetworkIconButton>
        )}
      </Stack>
    </Box>
  );
}
