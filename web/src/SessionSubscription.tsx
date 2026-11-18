import { Button, MenuItem, Stack, TextField, Typography } from "@mui/material";
import { useEffect, useState } from "react";
import { PREFERENCES_CHANGED } from "./aboutPreferences";
import { legacySubscriptionLabel, type SubscriptionDisplay, validSubscriptionDisplay } from "./subscriptionDisplay";
import { useNativeAccountUsage } from "./nativeAccountUsage";
import { loadUsageSnapshot, useStoreSelector } from "./store";
import { providerUsage, subscriptionQuotaHint } from "./usageLimits";
import { accountServes, providerDefault } from "./subscriptionAccounts";
import { accountControl, type ControlledAccounts, namedOnlyProvider, pickerUnavailableReason, validAccountControls } from "./accountControls";

interface Account { id: string; provider_id: string; label: string; enabled: boolean }
interface Accounts extends ControlledAccounts {
  registry: { accounts: Account[]; defaults: Record<string, string> }; display_preferences?: SubscriptionDisplay;
  session_admission: string; session_providers?: string[]; portable_scopes?: string[][];
}

export function providerSubscriptionEnabled(data: { session_admission: string; session_providers?: string[] } | undefined, provider: string): boolean {
  return data?.session_admission === "enabled" && data.session_providers?.includes(provider) === true;
}

/** Login state from the shared cached account observations (no Provider
 *  request). Only definite sign-in states disable a choice; transient Machine
 *  or quota conditions do not, and creation still rechecks the exact Machine. */
const LOGIN_REASONS: Record<string, string> = {
  native_login_outdated: "Sign in again: the Provider was updated",
  native_login_required: "Not signed in on a Machine",
  native_login_pending: "Sign-in not finished: finish or cancel it",
};
export function subscriptionLoginReason(
  observations: readonly { account_id: string; provider_id: string; reason?: string }[],
  provider: string,
  id: string,
): string | undefined {
  if (id === "default") return undefined;
  const reason = observations.find((item) => item.account_id === id && item.provider_id === provider)?.reason;
  return reason ? LOGIN_REASONS[reason] : undefined;
}

export function subscriptionAllowed(data: Accounts | undefined, provider: string, choice: string): boolean {
  if (!data) return false;
  // A named-only Provider (Antigravity) has no default account.
  if (namedOnlyProvider(data, provider) && (!choice || choice === "default")) return false;
  const id = choice || providerDefault(data, provider) || "default";
  if (id === "default") return true;
  return providerSubscriptionEnabled(data, provider) && data.registry.accounts.some((account) =>
    account.id === id && accountServes(data, account.provider_id, provider) && account.enabled
  );
}

function subscriptionUnavailableReason(data: Accounts | undefined, provider: string, id: string): string | undefined {
  if (id === "default") return undefined;
  const account = data?.registry.accounts.find((account) => account.id === id && accountServes(data, account.provider_id, provider));
  if (!account) return "Account unavailable";
  if (!account.enabled) return "Account disabled";
  if (data?.session_admission !== "enabled") return "Session access not enabled";
  if (!data.session_providers?.includes(provider)) return "Additional accounts not supported by this Provider";
  return undefined;
}

/** The choosable accounts of the picker. A named-only Provider lists only
 * its named accounts: admitted, signed-in, `ok` ones enabled, every other
 * one disabled with its reason, and nothing preselected. */
export function subscriptionChoices(data: Accounts | undefined, provider: string, loginReason: (id: string) => string | undefined, now = Date.now()) {
  const accounts = data?.registry.accounts.filter((account) => accountServes(data, account.provider_id, provider)) ?? [];
  const namedOnly = namedOnlyProvider(data, provider);
  const defaultId = providerDefault(data, provider) ?? "default";
  const legacyLabel = legacySubscriptionLabel(data?.display_preferences, provider);
  return [...namedOnly ? [] : [{ id: "default", label: legacyLabel }], ...accounts].map((account) => ({
    ...account,
    label: `${account.label}${data && !namedOnly && account.id === defaultId ? " · Default" : ""}`,
    reason: subscriptionUnavailableReason(data, provider, account.id) ??
      (namedOnly ? pickerUnavailableReason(accountControl(data, account.id), loginReason(account.id), now) : loginReason(account.id)),
  }));
}

/** One shared creation field for desktop and mobile; never selects a Machine. */
export function useSessionSubscription(active: boolean, provider: string, preferred?: string) {
  const [data, setData] = useState<Accounts>();
  const [error, setError] = useState(false);
  const [revision, refresh] = useState(0);
  const [selection, select] = useState({ provider: "", id: "" });
  const choice = selection.provider === provider ? selection.id : "";
  const native = useNativeAccountUsage();
  const loadNative = native.load;
  // One shared cached read (deduplicated across surfaces) when choosing.
  useEffect(() => { if (active) void loadNative(); }, [active, loadNative]);
  const snapshotLoaded = useStoreSelector((state) => state.usage !== null);
  useEffect(() => { if (active && !snapshotLoaded) void loadUsageSnapshot().catch(() => undefined); }, [active, snapshotLoaded]);
  const loginReason = (id: string) => subscriptionLoginReason(native.accounts, provider, id);
  // Cached observations only (the Service snapshot and named-account reads);
  // shown to help choose, never to pick or rotate an account automatically.
  const snapshot = useStoreSelector((state) => state.usage);
  const quota = (id: string) => subscriptionQuotaHint(id === "default"
    ? providerUsage(snapshot, provider)
    : native.accounts.find((item) => item.account_id === id && item.provider_id === provider)?.usage);
  // A usable preferred account (the directory's latest session) precedes the
  // registry default; an unavailable one falls back instead of blocking.
  const namedOnly = namedOnlyProvider(data, provider);
  const hinted = !namedOnly && preferred && subscriptionAllowed(data, provider, preferred) && !loginReason(preferred) ? preferred : "";
  // Resolve the default in this snapshot. Creation must submit the exact
  // account shown in the field, even if another client changes the default.
  // A named-only Provider's account is never preselected.
  const value = choice || hinted || (data && !namedOnly ? providerDefault(data, provider) || "default" : "");
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
            (!Array.isArray(next.session_providers) || !next.session_providers.every((id) => typeof id === "string"))) ||
          (next.portable_scopes !== undefined && (!Array.isArray(next.portable_scopes) ||
            !next.portable_scopes.every((scope) => Array.isArray(scope) && scope.every((id) => typeof id === "string")))) ||
          (next.named_only_providers !== undefined && (!Array.isArray(next.named_only_providers) ||
            !next.named_only_providers.every((id) => typeof id === "string"))) ||
          (next.account_controls !== undefined && !validAccountControls(next.account_controls))) throw new Error("invalid response");
        if (!abort.signal.aborted) setData(next);
      }).catch(() => { if (!abort.signal.aborted) setError(true); });
    return () => abort.abort();
  }, [active, revision]);
  return {
    value, data, error, provider,
    hinted: !choice && Boolean(hinted),
    /** The user picked a subscription instead of the default. */
    chosen: Boolean(choice),
    allowed: (subscriptionAllowed(data, provider, value) && !(namedOnly
      ? pickerUnavailableReason(accountControl(data, value), loginReason(value))
      : loginReason(value))) || (error && value === "default" && !namedOnly),
    loginReason,
    quota,
    select: (id: string) => select({ provider, id }),
    retry: () => refresh((current) => current + 1),
  };
}

export function SessionSubscription({ state, disabled, desktop, helperText }: {
  state: ReturnType<typeof useSessionSubscription>; disabled: boolean; desktop: boolean;
  helperText?: string | undefined;
}): React.JSX.Element {
  const selectedId = state.value;
  const namedOnly = namedOnlyProvider(state.data, state.provider);
  const choices = subscriptionChoices(state.data, state.provider, state.loginReason).map((account) => ({
    ...account, quota: namedOnly ? undefined : state.quota(account.id),
  }));
  const selected = choices.find((account) => account.id === selectedId);
  const placeholder = selectedId ? "Account unavailable" : state.error || (state.data && namedOnly) ? "Choose an account" : "Loading subscriptions…";
  return (
    <Stack spacing={0.5} data-session-subscription>
      <TextField select fullWidth size={desktop ? "small" : "medium"} label="Subscription"
        slotProps={{ select: { displayEmpty: true, renderValue: () => selected?.label ?? placeholder }, inputLabel: { shrink: true } }}
        value={selectedId} disabled={disabled || (!state.data && !state.error)}
        error={state.error || Boolean(state.data && !state.allowed && (selectedId || !namedOnly))}
        helperText={state.error ? "Could not load subscriptions." : !state.data ? "Loading subscriptions…" : !state.allowed ? selected?.reason ?? (namedOnly && !selectedId ? "Choose a signed-in, admitted account." : "Account unavailable") : helperText ?? selected?.quota ?? "Independent of the selected Machine."}
        onChange={(event) => state.select(event.target.value)}>
        {!selected && <MenuItem value={selectedId} disabled>{placeholder}</MenuItem>}
        {choices.map((account) => <MenuItem key={account.id} value={account.id} disabled={Boolean(account.reason)}>
          <Stack sx={{ minWidth: 0, whiteSpace: "normal" }}>
            <span>{account.label}</span>
            {(account.reason ?? account.quota) && <Typography variant="caption" color={account.reason ? undefined : "text.secondary"}>{account.reason ?? account.quota}</Typography>}
          </Stack>
        </MenuItem>)}
      </TextField>
      {state.error && <Button onClick={state.retry} disabled={disabled}>Retry</Button>}
    </Stack>
  );
}
