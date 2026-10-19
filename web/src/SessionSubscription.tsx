import { Button, MenuItem, Stack, TextField } from "@mui/material";
import { useEffect, useState } from "react";

interface Account { id: string; provider_id: string; label: string; enabled: boolean }
interface Accounts { registry: { accounts: Account[]; defaults: Record<string, string> }; session_admission: string; session_providers?: string[] }

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

/** One shared creation field for desktop and mobile; never selects a Machine. */
export function useSessionSubscription(active: boolean, provider: string) {
  const [data, setData] = useState<Accounts>();
  const [error, setError] = useState(false);
  const [revision, refresh] = useState(0);
  const [selection, select] = useState({ provider: "", id: "" });
  const value = selection.provider === provider ? selection.id : "";
  useEffect(() => {
    if (!active) { setData(undefined); select({ provider: "", id: "" }); return; }
    const abort = new AbortController();
    setError(false);
    setData(undefined);
    void fetch("/api/provider-accounts", { cache: "no-store", signal: abort.signal })
      .then(async (response) => {
        if (!response.ok) throw new Error("unavailable");
        const next = await response.json() as Accounts;
        if (!Array.isArray(next.registry?.accounts) || !next.registry.defaults ||
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

export function SessionSubscription({ state, disabled, desktop }: {
  state: ReturnType<typeof useSessionSubscription>; disabled: boolean; desktop: boolean;
}): React.JSX.Element {
  const accounts = state.data?.registry.accounts.filter((account) => account.provider_id === state.provider) ?? [];
  const defaultId = state.data?.registry.defaults[state.provider] ?? "default";
  const defaultLabel = accounts.find((account) => account.id === defaultId)?.label ?? (defaultId === "default" ? "Existing account" : "Unavailable");
  return (
    <Stack spacing={0.5}>
      <TextField select fullWidth size={desktop ? "small" : "medium"} label="Subscription"
        value={state.value} disabled={disabled || (!state.data && !state.error)}
        error={state.error || Boolean(state.data && !state.allowed)}
        helperText={state.error ? "Could not load subscriptions." : !state.data ? "Loading subscriptions…" : !state.allowed ? "This subscription is not available for new sessions." : "Independent of the selected Machine."}
        onChange={(event) => state.select(event.target.value)}>
        <MenuItem value="" disabled={state.error}>Account default · {defaultLabel}</MenuItem>
        <MenuItem value="default">Existing account</MenuItem>
        {accounts.map((account) => <MenuItem key={account.id} value={account.id} disabled={!account.enabled || !providerSubscriptionEnabled(state.data, account.provider_id)}>
          {account.label}{!account.enabled ? " · Disabled" : !providerSubscriptionEnabled(state.data, account.provider_id) ? " · Unavailable" : ""}
        </MenuItem>)}
      </TextField>
      {state.error && <Button onClick={state.retry} disabled={disabled}>Retry</Button>}
    </Stack>
  );
}
