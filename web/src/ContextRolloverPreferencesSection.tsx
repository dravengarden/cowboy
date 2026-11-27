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
  MIN_CONTEXT_TOKEN_PRESETS,
  providerIdleChoices,
  providerRolloverDescription,
  useContextRolloverPreferences,
  withProviderIdleMinutes,
} from "./contextRolloverPreferences";

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
  onSelectIdle,
}: {
  provider: ContextRolloverProviderChoice;
  values: ContextRolloverValues;
  disabled: boolean;
  onSelectIdle: (nextIdle: Record<string, number | null>) => void;
}): React.JSX.Element {
  const effective = effectiveProviderIdleMinutes(values, provider);
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
      <Box sx={{ flexShrink: 0 }}>
        {provider.policy_restricted ? (
          <Chip
            size="small"
            variant="outlined"
            label="Disabled by policy"
            sx={{ fontSize: "0.72rem" }}
          />
        ) : (
          <Select
            size="small"
            value={effective === null ? "off" : String(effective)}
            disabled={disabled}
            inputProps={{ "aria-label": `${provider.name} cache renewal threshold` }}
            onChange={(event): void => {
              const raw = String(event.target.value);
              const nextMinutes = raw === "off" ? null : Number(raw);
              if (nextMinutes !== null && !Number.isFinite(nextMinutes)) return;
              onSelectIdle(withProviderIdleMinutes(values, provider, nextMinutes));
            }}
            sx={{ minWidth: 164 }}
          >
            {choices.map((option) => (
              <MenuItem key={option.value} value={option.value}>
                {option.label}
              </MenuItem>
            ))}
          </Select>
        )}
      </Box>
    </Stack>
  );
}

/** Global and per-Provider idle prompt-cache renewal preferences (Settings → Agent). */
export function ContextRolloverPreferencesSection(): React.JSX.Element {
  const { data, error, saving, pending, save, retry, discard } =
    useContextRolloverPreferences();
  const values = data?.preferences.values;
  const providers = data?.providers ?? [];

  const tokenChoices = React.useMemo(() => {
    const set = new Set<number>(MIN_CONTEXT_TOKEN_PRESETS);
    if (values?.min_context_tokens) set.add(values.min_context_tokens);
    return [...set].sort((a, b) => a - b);
  }, [values?.min_context_tokens]);

  const customized =
    values !== undefined &&
    (!values.enabled ||
      values.min_context_tokens !== DEFAULT_MIN_CONTEXT_TOKENS ||
      Object.keys(values.idle_minutes).length > 0);
  const busy = saving || Boolean(pending);

  return (
    <Box
      data-context-rollover-preferences
      sx={{ pt: 1.5, mt: 1, borderTop: 1, borderColor: "divider" }}
    >
      <Stack direction="row" alignItems="center" justifyContent="space-between" spacing={1}>
        <Typography
          variant="overline"
          color="text.secondary"
          sx={{ letterSpacing: 0.8, lineHeight: 1.6 }}
        >
          Prompt cache renewal
        </Typography>
        {customized && (
          <Button
            size="small"
            disabled={busy}
            onClick={(): void => {
              void save({
                enabled: true,
                min_context_tokens: DEFAULT_MIN_CONTEXT_TOKENS,
                idle_minutes: {},
              });
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
                void save({ enabled: checked });
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
                  void save({ min_context_tokens: parsed });
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
              onSelectIdle={(idle_minutes): void => {
                void save({ idle_minutes });
              }}
            />
          ))}
        </Stack>
      )}
    </Box>
  );
}
