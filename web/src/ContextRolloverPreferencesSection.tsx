import React from "react";
import {
  Box,
  Button,
  Chip,
  MenuItem,
  Select,
  Stack,
  Switch,
  Typography,
} from "@mui/material";
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
import { useStoreSelector } from "./store";

function formatContextTokens(tokens: number): string {
  if (tokens >= 1_000_000 && tokens % 1_000_000 === 0) {
    return `${tokens / 1_000_000}M`;
  }
  return `${Math.round(tokens / 1_000)}k`;
}

function ProviderRolloverRow({
  provider,
  values,
  disabled,
  onSelect,
}: {
  provider: ContextRolloverProviderChoice;
  values: ContextRolloverValues;
  disabled: boolean;
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
        <Typography variant="body2">{provider.name}</Typography>
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

/** Per-Provider prompt-cache renewal policy editor inside Settings -> Agent. */
export function ContextRolloverPreferencesSection(): React.JSX.Element {
  const connected = useStoreSelector((state) => state.connected);
  const { data, error, saving, pending, apply, retry, discard } =
    useContextRolloverPreferences(connected);
  const values = data?.preferences.values;
  const providers = data?.providers ?? [];
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
              onChange={(_, checked): void => {
                void apply({ kind: "enabled", enabled: checked });
              }}
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
                if (Number.isFinite(parsed) && parsed > 0) {
                  void apply({ kind: "min_context_tokens", min_context_tokens: parsed });
                }
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

          {providers.map((provider) => (
            <ProviderRolloverRow
              key={provider.id}
              provider={provider}
              values={values}
              disabled={busy || !values.enabled}
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
