import React, { useEffect, useMemo, useState } from "react";
import {
  Box,
  Button,
  Chip,
  MenuItem,
  Select,
  Stack,
  Switch,
  Tooltip,
  Typography,
} from "@mui/material";
import {
  type AccountCacheMetricsResponse,
  fetchAccountCacheMetrics,
  formatSharePercent,
} from "./cacheMetricsApi";
import {
  type ContextRolloverProviderChoice,
  type ContextRolloverValues,
  DEFAULT_MIN_CONTEXT_TOKENS,
  effectiveProviderIdleMinutes,
  hasCustomRolloverPreferences,
  MIN_CONTEXT_TOKEN_PRESETS,
  providerIdleChoices,
  providerIdleSelectionValue,
  providerRolloverDescription,
  type ProviderIdleSelection,
  useContextRolloverPreferences,
} from "./contextRolloverPreferences";
import type { MetricTone } from "./InfoMetricVisuals";
import {
  aggregateProviderCacheShare,
  type ProviderSevenDayCacheSummary,
  shareMetricTone,
} from "./sessionCacheUi";
import { useStoreSelector } from "./store";
import { providerDefault, useSubscriptionAccounts } from "./subscriptionAccounts";
import { usageCacheIntervalLabel } from "./usageHostMap";

function formatContextTokens(tokens: number): string {
  if (tokens >= 1_000_000 && tokens % 1_000_000 === 0) {
    return `${tokens / 1_000_000}M`;
  }
  return `${Math.round(tokens / 1_000)}k`;
}

function shareChipColor(tone: MetricTone): "success" | "info" | "warning" | "default" {
  if (tone === "success" || tone === "info" || tone === "warning") return tone;
  return "default";
}

function useProviderSevenDayCacheStats(
  connected: boolean,
  providers: readonly ContextRolloverProviderChoice[],
): ReadonlyMap<string, ProviderSevenDayCacheSummary> {
  const { data: accountsSnapshot } = useSubscriptionAccounts(connected);
  const [byAccount, setByAccount] = useState<ReadonlyMap<string, AccountCacheMetricsResponse>>(
    () => new Map(),
  );

  const providerAccountIds = useMemo(() => {
    const map = new Map<string, string[]>();
    if (!accountsSnapshot) return map;
    const enabledAccounts = accountsSnapshot.registry.accounts.filter(
      (account) => account.enabled && !account.revoking,
    );
    for (const provider of providers) {
      const owned = enabledAccounts
        .filter((account) => account.provider_id === provider.id)
        .map((account) => account.id);
      if (owned.length > 0) {
        map.set(provider.id, owned);
      } else {
        const fallback = providerDefault(accountsSnapshot, provider.id);
        if (fallback) map.set(provider.id, [fallback]);
      }
    }
    return map;
  }, [accountsSnapshot, providers]);

  const uniqueAccountKey = useMemo(() => {
    const ids = new Set<string>();
    for (const list of providerAccountIds.values()) {
      for (const id of list) ids.add(id);
    }
    return [...ids].sort().join(",");
  }, [providerAccountIds]);

  useEffect(() => {
    if (!connected || !uniqueAccountKey) return;
    const ids = uniqueAccountKey.split(",");
    const controller = new AbortController();
    void Promise.allSettled(
      ids.map(async (id) => ({
        id,
        res: await fetchAccountCacheMetrics(id, "7d", controller.signal),
      })),
    ).then((results) => {
      if (controller.signal.aborted) return;
      const next = new Map<string, AccountCacheMetricsResponse>();
      for (const result of results) {
        if (result.status === "fulfilled") {
          next.set(result.value.id, result.value.res);
        }
      }
      setByAccount(next);
    });
    return (): void => controller.abort();
  }, [connected, uniqueAccountKey]);

  return useMemo(() => {
    const stats = new Map<string, ProviderSevenDayCacheSummary>();
    for (const [providerId, accountIds] of providerAccountIds.entries()) {
      const responses = accountIds
        .map((id) => byAccount.get(id))
        .filter((item): item is AccountCacheMetricsResponse => item !== undefined);
      const summary = aggregateProviderCacheShare(responses);
      if (summary) stats.set(providerId, summary);
    }
    return stats;
  }, [providerAccountIds, byAccount]);
}

function ProviderRolloverRow({
  provider,
  values,
  disabled,
  sevenDay,
  onSelect,
}: {
  provider: ContextRolloverProviderChoice;
  values: ContextRolloverValues;
  disabled: boolean;
  sevenDay?: ProviderSevenDayCacheSummary | undefined;
  onSelect: (selection: ProviderIdleSelection) => void;
}): React.JSX.Element {
  const effective = effectiveProviderIdleMinutes(values, provider);
  const selectedValue = providerIdleSelectionValue(values, provider);
  const choices = providerIdleChoices(provider, effective);
  return (
    <Stack
      data-settings-row
      data-rollover-provider={provider.id}
      direction="row"
      alignItems="center"
      justifyContent="space-between"
      spacing={2}
      sx={{ py: 0.75, minHeight: 44 }}
    >
      <Stack sx={{ minWidth: 0 }}>
        <Stack direction="row" alignItems="center" spacing={0.75} flexWrap="wrap" useFlexGap>
          <Typography variant="body2">{provider.name}</Typography>
          {provider.keepalive_independent && (
            <Tooltip title="Supports optional per-session paid gateway keepalive (default 5h30m adaptive interval; actual attempt and age caps are reported per session). On supported Machines, automatically hands off to idle renewal once keepalive expires; older Machines require manual hibernate or Clear after keepalive ends.">
              <Chip
                size="small"
                variant="outlined"
                data-provider-keepalive-cadence={provider.id}
                label={`Keepalive · ${usageCacheIntervalLabel(provider.id) || "5h30m"}`}
                sx={{ height: 19, fontSize: "0.68rem", color: "text.secondary" }}
              />
            </Tooltip>
          )}
          {sevenDay && sevenDay.cachedShare !== null && (
            <Tooltip
              title={`Observed ${sevenDay.observedTurns.toLocaleString()} turn${sevenDay.observedTurns === 1 ? "" : "s"} across registered account(s) in the last 7 days`}
            >
              <Chip
                size="small"
                variant="outlined"
                data-provider-7d-cache-share={provider.id}
                color={shareChipColor(shareMetricTone(sevenDay.cachedShare))}
                label={`7d · ${formatSharePercent(sevenDay.cachedShare)}`}
                sx={{ height: 19, fontSize: "0.68rem", fontWeight: 600 }}
              />
            </Tooltip>
          )}
        </Stack>
        <Typography variant="caption" color="text.secondary">
          {providerRolloverDescription(provider)}
        </Typography>
      </Stack>
      {provider.policy_restricted ? (
        <Chip
          size="small"
          label="Disabled by policy"
          variant="outlined"
          sx={{ flexShrink: 0 }}
        />
      ) : (
        <Select
          size="small"
          value={selectedValue}
          disabled={disabled}
          inputProps={{ "aria-label": `${provider.name} idle renewal threshold` }}
          onChange={(event): void => {
            const raw = String(event.target.value);
            if (raw === "default" || raw === "off") {
              onSelect(raw);
              return;
            }
            const parsed = Number(raw);
            if (Number.isFinite(parsed) && parsed >= 1) {
              onSelect(parsed);
            }
          }}
          sx={{ minWidth: 184, flexShrink: 0 }}
        >
          {choices.map((option) => (
            <MenuItem key={option.value} value={option.value}>
              {option.label}
            </MenuItem>
          ))}
        </Select>
      )}
    </Stack>
  );
}

function RolloverGlobalControls({
  values,
  busy,
  tokenChoices,
  onEnabledChange,
  onMinTokensChange,
}: {
  values: ContextRolloverValues;
  busy: boolean;
  tokenChoices: readonly number[];
  onEnabledChange: (enabled: boolean) => void;
  onMinTokensChange: (tokens: number) => void;
}): React.JSX.Element {
  return (
    <>
      <Stack
        data-settings-row
        direction="row"
        alignItems="center"
        justifyContent="space-between"
        spacing={2}
        sx={{ py: 0.75, minHeight: 44 }}
      >
        <Stack sx={{ minWidth: 0 }}>
          <Typography variant="body2">Fresh context after cache expiry</Typography>
          <Typography variant="caption" color="text.secondary">
            Master switch for all eligible Providers below
          </Typography>
        </Stack>
        <Switch
          size="small"
          checked={values.enabled}
          disabled={busy}
          inputProps={{ "aria-label": "Enable prompt-cache context renewal" }}
          onChange={(_, checked): void => onEnabledChange(checked)}
        />
      </Stack>

      <Stack
        data-settings-row
        direction="row"
        alignItems="center"
        justifyContent="space-between"
        spacing={2}
        sx={{ py: 0.75, minHeight: 44 }}
      >
        <Stack sx={{ minWidth: 0 }}>
          <Typography variant="body2">Minimum context size</Typography>
          <Typography variant="caption" color="text.secondary">
            Smaller contexts resume directly without renewal
          </Typography>
        </Stack>
        <Select
          size="small"
          value={String(values.min_context_tokens)}
          disabled={busy || !values.enabled}
          inputProps={{ "aria-label": "Minimum context size for renewal" }}
          onChange={(event): void => {
            const parsed = Number(event.target.value);
            if (Number.isFinite(parsed) && parsed > 0) onMinTokensChange(parsed);
          }}
          sx={{ minWidth: 148 }}
        >
          {tokenChoices.map((tokens) => (
            <MenuItem key={tokens} value={String(tokens)}>
              {formatContextTokens(tokens)}
              {tokens === DEFAULT_MIN_CONTEXT_TOKENS ? " · default" : ""}
            </MenuItem>
          ))}
        </Select>
      </Stack>
    </>
  );
}

/** Per-Provider prompt-cache renewal policy editor inside Settings -> Agent. */
export function ContextRolloverPreferencesSection(): React.JSX.Element {
  const connected = useStoreSelector((state) => state.connected);
  const { data, error, saving, pending, apply, retry, discard } =
    useContextRolloverPreferences(connected);
  const values = data?.preferences.values;
  const providers = useMemo(() => data?.providers ?? [], [data?.providers]);
  const sevenDayStats = useProviderSevenDayCacheStats(connected, providers);
  const busy = saving || Boolean(pending);

  const tokenChoices = Array.from(
    new Set(
      values
        ? [...MIN_CONTEXT_TOKEN_PRESETS, values.min_context_tokens]
        : MIN_CONTEXT_TOKEN_PRESETS,
    ),
  ).sort((a, b) => a - b);

  const customized = values ? hasCustomRolloverPreferences(values) : false;

  return (
    <Box
      data-context-rollover-preferences
      sx={{ pt: 2.5, mt: 2, borderTop: 1, borderColor: "divider" }}
    >
      <Stack
        direction="row"
        alignItems="center"
        justifyContent="space-between"
        spacing={1}
        sx={{ pb: 0.75 }}
      >
        <Typography
          variant="overline"
          sx={{ letterSpacing: "0.08em", color: "text.secondary", fontWeight: 700 }}
        >
          Prompt cache renewal
        </Typography>
        {customized && (
          <Button
            size="small"
            disabled={busy}
            onClick={(): void => {
              void apply({ kind: "reset_defaults" });
            }}
          >
            Reset defaults
          </Button>
        )}
      </Stack>

      <Typography variant="caption" color="text.secondary" sx={{ display: "block", pb: 0.5 }}>
        When an idle session wakes after its Provider prompt cache expired, start a fresh native
        context with a compact digest instead of paying a cold full-context write. Each session can
        still override this in its Settings.
      </Typography>

      {error && (
        <Stack direction="row" alignItems="center" spacing={1} sx={{ py: 0.5, flexWrap: "wrap" }}>
          <Typography variant="caption" color="error">
            {error}
          </Typography>
          <Button size="small" disabled={saving} onClick={(): void => void retry()}>
            Retry
          </Button>
          {pending && (
            <Button size="small" disabled={saving} onClick={discard}>
              Discard
            </Button>
          )}
        </Stack>
      )}

      {values && (
        <Stack spacing={0.25}>
          <RolloverGlobalControls
            values={values}
            busy={busy}
            tokenChoices={tokenChoices}
            onEnabledChange={(enabled): void => {
              void apply({ kind: "enabled", enabled });
            }}
            onMinTokensChange={(min_context_tokens): void => {
              void apply({ kind: "min_context_tokens", min_context_tokens });
            }}
          />

          {providers.map((provider) => (
            <ProviderRolloverRow
              key={provider.id}
              provider={provider}
              values={values}
              disabled={busy || !values.enabled}
              sevenDay={sevenDayStats.get(provider.id)}
              onSelect={(selection): void => {
                void apply({ kind: "provider_idle", providerId: provider.id, selection });
              }}
            />
          ))}
        </Stack>
      )}
    </Box>
  );
}
