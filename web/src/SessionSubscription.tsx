import { Button, MenuItem, Stack, TextField, Typography } from "@mui/material";
import { useEffect, useState } from "react";
import { PREFERENCES_CHANGED } from "./aboutPreferences";
import { legacySubscriptionLabel, type SubscriptionDisplay, validSubscriptionDisplay } from "./subscriptionDisplay";

interface Account { id: string; provider_id: string; label: string; enabled: boolean }
interface Accounts { registry: { accounts: Account[]; defaults: Record<string, string> }; display_preferences?: SubscriptionDisplay; session_admission: string; session_providers?: string[] }

export function providerSubscriptionEnabled(data: { session_admission: string; session_providers?: string[] } | undefined, provider: string): boolean {
  return data?.session_admission === "enabled" && data.session_providers?.includes(provider) === true;
}

export function subscriptionAllowed(data: Accounts | undefined, provider: string, choice: string): boolean {
  if (!data) return false;
  const id = choice || data.registry.defaults[provider] || "default";
  if (id === "default") return true;
  return providerSubscriptionEnabled(data, provider) && data.registry.accounts.some((account) =>
    account.id === id && account.provider_id === provider && account.enabled
  );
}

function subscriptionUnavailableReason(data: Accounts | undefined, provider: string, id: string): string | undefined {
  if (id === "default") return undefined;
  const account = data?.registry.accounts.find((account) => account.id === id && account.provider_id === provider);
  if (!account) return "Account unavailable";
  if (!account.enabled) return "Account disabled";
  if (data?.session_admission !== "enabled") return "Session access not enabled";
  if (!data.session_providers?.includes(provider)) return "Additional accounts not supported by this Provider";
  return undefined;
}

/** One shared creation field for desktop and mobile; never selects a Machine. */
export function useSessionSubscription(active: boolean, provider: string) {
  const [data, setData] = useState<Accounts>();
  const [error, setError] = useState(false);
  const [revision, refresh] = useState(0);
  const [selection, select] = useState({ provider: "", id: "" });
  const choice = selection.provider === provider ? selection.id : "";
  // Resolve the default in this snapshot. Creation must submit the exact
  // account shown in the field, even if another client changes the default.
  const value = choice || (data ? data.registry.defaults[provider] || "default" : "");
  useEffect(() => {
    if (!active) return;
    const changed = (event: Event) => {
      const scopes = (event as CustomEvent<Record<string, number> | undefined>).detail;
      if (!scopes || Object.hasOwn(scopes, "provider_accounts") || Object.hasOwn(scopes, "subscriptions")) refresh((current) => current + 1);
    };
    window.addEventListener(PREFERENCES_CHANGED, changed);
    return () => window.removeEventListener(PREFERENCES_CHANGED, changed);
  }, [active]);
  useEffect(() => {
    if (!active) { setData(undefined); select({ provider: "", id: "" }); return; }
    const abort = new AbortController();
    setError(false);
    setData(undefined);
    void fetch("/api/provider-accounts", { cache: "no-store", signal: abort.signal })
      .then(async (response) => {
        if (!response.ok) throw new Error("unavailable");
        const next = await response.json() as Accounts;
        if ((next.display_preferences !== undefined && !validSubscriptionDisplay(next.display_preferences)) ||
          !Array.isArray(next.registry?.accounts) || !next.registry.defaults ||
          typeof next.registry.defaults !== "object" || Array.isArray(next.registry.defaults) ||
          !Object.values(next.registry.defaults).every((id) => typeof id === "string") ||
          !next.registry.accounts.every((account) => account && typeof account.id === "string" &&
            typeof account.provider_id === "string" && typeof account.label === "string" && typeof account.enabled === "boolean") ||
          typeof next.session_admission !== "string" || (next.session_providers !== undefined &&
            (!Array.isArray(next.session_providers) || !next.session_providers.every((id) => typeof id === "string")))) throw new Error("invalid response");
        if (!abort.signal.aborted) setData(next);
      }).catch(() => { if (!abort.signal.aborted) setError(true); });
    return () => abort.abort();
  }, [active, revision]);
  return {
    value, data, error, provider,
    allowed: subscriptionAllowed(data, provider, value) || (error && value === "default"),
    select: (id: string) => select({ provider, id }),
    retry: () => refresh((current) => current + 1),
  };
}

export function SessionSubscription({ state, disabled, desktop, helperText }: {
  state: ReturnType<typeof useSessionSubscription>; disabled: boolean; desktop: boolean;
  helperText?: string;
}): React.JSX.Element {
  const accounts = state.data?.registry.accounts.filter((account) => account.provider_id === state.provider) ?? [];
  const defaultId = state.data?.registry.defaults[state.provider] ?? "default";
  const legacyLabel = legacySubscriptionLabel(state.data?.display_preferences, state.provider);
  const selectedId = state.value;
  const choices = [{ id: "default", label: legacyLabel }, ...accounts].map((account) => ({
    ...account,
    label: `${account.label}${state.data && account.id === defaultId ? " · Default" : ""}`,
    reason: subscriptionUnavailableReason(state.data, state.provider, account.id),
  }));
  const selected = choices.find((account) => account.id === selectedId);
  const placeholder = selectedId ? "Account unavailable" : state.error ? "Choose a subscription" : "Loading subscriptions…";
  return (
    <Stack spacing={0.5} data-session-subscription>
      <TextField select fullWidth size={desktop ? "small" : "medium"} label="Subscription"
        slotProps={{ select: { displayEmpty: true, renderValue: () => selected?.label ?? placeholder }, inputLabel: { shrink: true } }}
        value={selectedId} disabled={disabled || (!state.data && !state.error)}
        error={state.error || Boolean(state.data && !state.allowed)}
        helperText={state.error ? "Could not load subscriptions." : !state.data ? "Loading subscriptions…" : !state.allowed ? selected?.reason ?? "Account unavailable" : helperText ?? "Independent of the selected Machine."}
        onChange={(event) => state.select(event.target.value)}>
        {!selected && <MenuItem value={selectedId} disabled>{placeholder}</MenuItem>}
        {choices.map((account) => <MenuItem key={account.id} value={account.id} disabled={Boolean(account.reason)}>
          <Stack sx={{ minWidth: 0, whiteSpace: "normal" }}>
            <span>{account.label}</span>
            {account.reason && <Typography variant="caption">{account.reason}</Typography>}
          </Stack>
        </MenuItem>)}
      </TextField>
      {state.error && <Button onClick={state.retry} disabled={disabled}>Retry</Button>}
    </Stack>
  );
}
