import { Alert, Button, MenuItem, Paper, Stack, TextField, Typography } from "@mui/material";
import type { ReactNode } from "react";
import { useEffect, useRef, useState } from "react";
import { latestProviderEntries, useProviderCatalog } from "./providerCatalog";
import { productSessionSignal } from "./productSessionEnd";
import { productSyncPrincipal } from "./productSyncIdentity";
import { PREFERENCES_CHANGED } from "./aboutPreferences";
import { providerSubscriptionEnabled } from "./SessionSubscription";
import { SubscriptionLogins } from "./SubscriptionLogins";
import { SubscriptionApiKey } from "./SubscriptionApiKey";
import { SubscriptionRevocation } from "./SubscriptionRevocation";
import { SubscriptionBrowserActions } from "./SubscriptionBrowserActions";
import { type BrowserShareChange, BrowserShareSettings } from "./machineBrowser/BrowserShareSettings";
import { sharedBrowser } from "./machineBrowser/shareRules";
import { browserErrorText } from "./machineBrowser/api";
import { ProviderAccountIdentity, providerAccountCardSx } from "./ProviderAccountIdentity";
import { validSubscriptionLabel } from "./subscriptionDisplay";
import { accountDefaults, type SubscriptionSnapshot as Snapshot, validSubscriptionSnapshot, withoutLegacyShareMembers } from "./subscriptionAccounts";
import { accountControl } from "./accountControls";
import { AccountControlsPanel } from "./AccountControlsPanel";
import { useNativeAccountUsage } from "./nativeAccountUsage";
export { validSubscriptionSnapshot } from "./subscriptionAccounts";
export { validSubscriptionLabel } from "./subscriptionDisplay";

type Change = { type: "create"; provider_id: string; label: string }
  | { type: "rename"; id: string; label: string }
  | { type: "set_enabled"; id: string; enabled: boolean }
  | { type: "set_default"; provider_id: string; id: string }
  | { type: "set_browser_enabled"; id: string; enabled: boolean }
  | BrowserShareChange;
interface Pending { operation: string; dataset: string; expected_revision: number; change: Change }

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
function revision(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}
function text(value: unknown): value is string { return typeof value === "string" && value.length > 0; }

export function validPendingSubscription(value: unknown): value is Pending {
  if (!record(value) || !text(value.operation) || !text(value.dataset) || !revision(value.expected_revision) || !record(value.change)) return false;
  if (Object.keys(value).some((key) => !["operation", "dataset", "expected_revision", "change"].includes(key))) return false;
  const change = value.change;
  const fields = change.type === "create" ? ["type", "provider_id", "label"]
    : change.type === "rename" ? ["type", "id", "label"]
    : change.type === "set_enabled" ? ["type", "id", "enabled"]
    : change.type === "set_browser_enabled" ? ["type", "id", "enabled"]
    : change.type === "set_default" ? ["type", "provider_id", "id"]
    : change.type === "create_browser_share" ? ["type", "label"]
    : change.type === "rename_browser_share" ? ["type", "share_id", "label"]
    : change.type === "delete_browser_share" ? ["type", "share_id"]
    : change.type === "set_browser_share" ? ["type", "member", "share_id"] : [];
  if (fields.length === 0 || Object.keys(change).some((key) => !fields.includes(key))) return false;
  switch (change.type) {
    case "create": return text(change.provider_id) && typeof change.label === "string" && validSubscriptionLabel(change.label);
    case "rename": return text(change.id) && typeof change.label === "string" && validSubscriptionLabel(change.label);
    case "set_enabled": case "set_browser_enabled": return text(change.id) && typeof change.enabled === "boolean";
    case "set_default": return text(change.provider_id) && text(change.id);
    case "create_browser_share": return typeof change.label === "string" && validSubscriptionLabel(change.label);
    case "rename_browser_share": return text(change.share_id) && typeof change.label === "string" && validSubscriptionLabel(change.label);
    case "delete_browser_share": return text(change.share_id);
    case "set_browser_share": return validShareMember(change.member) && (change.share_id === null || text(change.share_id));
    default: return false;
  }
}

function validShareMember(value: unknown): boolean {
  return record(value) && Object.keys(value).length === 1 && text(value.account_id);
}

/** A pending change a Web release from before the legacy card retirement
 * saved (a legacy label edit, a card browser switch or a card share
 * membership). Its effect is retired whether or not it committed: the
 * Controller ignores legacy labels and removes every card browser and card
 * share member at start. It is discarded, never retried. */
export function obsoletePendingSubscription(value: unknown): boolean {
  if (!record(value) || !record(value.change)) return false;
  const change = value.change;
  return change.type === "set_legacy_labels" || change.type === "set_legacy_browser_enabled" ||
    (change.type === "set_browser_share" && record(change.member) && text(change.member.card));
}

/** Only explicit domain rejections establish that this mutation did not commit. */
export function subscriptionWriteRejected(status: number, code: string | undefined): boolean {
  return status === 422 && ["invalid_account", "account_capacity", "account_is_default", "invalid_provider", "account_unavailable", "invalid_preferences",
    "account_not_found", "account_revoking", "invalid_argument", "invalid_browser_share", "browser_share_capacity", "browser_share_not_found",
    "browser_share_family_conflict", "browser_share_family_unknown", "browser_share_family_bound", "browser_share_family_drift",
    "browser_share_changed", "browser_share_machine_unsupported", "browser_share_machine_unknown",
    "browser_share_reset_pending", "named_account_required"].includes(code ?? "");
}

/** The owner-facing explanation of a definitive refusal. */
export function subscriptionRejectionText(code: string): string {
  return code.startsWith("browser_share_")
    ? browserErrorText(code)
    : `Change rejected (${code}). Review the list before trying again.`;
}

export interface SubscriptionCards {
  named: { id: string; provider: string; content: ReactNode }[];
  /** Providers with no default (Service) account: named-only Providers and
   * every retired legacy card's Provider. Only their named cards render. */
  namedOnly: string[];
}

/** One account surface; signed Provider UI continues to own credential actions. */
export function SubscriptionManagement({ renderAccounts }: { renderAccounts?: (cards: SubscriptionCards) => ReactNode } = {}): React.JSX.Element {
  const { catalog } = useProviderCatalog();
  const providers = latestProviderEntries(catalog?.providers ?? []);
  const [snapshot, setSnapshot] = useState<Snapshot>();
  const [error, setError] = useState("");
  const [pending, setPending] = useState<Pending>();
  const [busy, setBusy] = useState(false);
  const [journalReady, setJournalReady] = useState(false);
  const [names, setNames] = useState<Record<string, string>>({});
  const [provider, setProvider] = useState("");
  const [label, setLabel] = useState("");
  const [adding, setAdding] = useState(false);
  const [editing, setEditing] = useState<string[]>([]);
  const native = useNativeAccountUsage(snapshot?.registry.revision);
  const lock = useRef(false);
  const alive = useRef(false);
  const loading = useRef(0);
  const owner = productSyncPrincipal();
  const journalKey = `cowboy:subscription-write:${owner ?? "unbound"}`;
  const live = () => alive.current && !productSessionSignal().aborted && owner === productSyncPrincipal();
  async function load() {
    const sequence = ++loading.current;
    try {
      const response = await fetch("/api/provider-accounts", { cache: "no-store", signal: productSessionSignal() });
      if (!response.ok) throw new Error("Could not load subscriptions.");
      const value: unknown = await response.json();
      if (!live() || sequence !== loading.current) return;
      if (!validSubscriptionSnapshot(value, owner)) {
        throw new Error("Account changed. Reload before editing subscriptions.");
      }
      setSnapshot((old) => !old || old.dataset.dataset_id !== value.dataset.dataset_id ||
        value.registry.revision >= old.registry.revision ? withoutLegacyShareMembers(value) : old);
    } catch (cause) { if (live() && sequence === loading.current) setError(String(cause)); }
  }
  useEffect(() => {
    alive.current = true;
    try {
      const saved = sessionStorage.getItem(journalKey);
      if (saved) {
        const value: unknown = JSON.parse(saved);
        if (obsoletePendingSubscription(value)) {
          sessionStorage.removeItem(journalKey);
          setError("A pending change to a retired legacy default card was discarded. Legacy cards are retired; nothing needs to be retried.");
        } else if (!validPendingSubscription(value)) throw new Error("Invalid pending subscription change.");
        else {
          setPending(value);
          setError("A previous change needs reconciliation. Retry sends the same operation.");
        }
      }
      setJournalReady(true);
    } catch { setError("Could not read the pending subscription change. Local storage must be available before editing."); }
    void load();
    const refresh = (event: Event) => {
      const scopes = (event as CustomEvent<Record<string, number> | undefined>).detail;
      if (!scopes || Object.hasOwn(scopes, "provider_accounts")) void load();
    };
    window.addEventListener(PREFERENCES_CHANGED, refresh);
    return () => { alive.current = false; ++loading.current; window.removeEventListener(PREFERENCES_CHANGED, refresh); };
    // This panel belongs to one authenticated product mount.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  /** Resolves `true` only once the change is committed. */
  async function commit(input: Pending): Promise<boolean> {
    if (lock.current || !snapshot || !journalReady || !live()) return false;
    if (input.dataset !== snapshot.dataset.dataset_id) {
      setError("This pending change belongs to another dataset. It will not be submitted.");
      return false;
    }
    lock.current = true;
    setBusy(true);
    try {
      // Persist before sending so remounting or reloading this tab cannot silently
      // replace an uncertain create with a second operation.
      sessionStorage.setItem(journalKey, JSON.stringify(input));
      setPending(input);
      const response = await fetch("/api/provider-accounts", {
        method: "PUT", headers: { "Content-Type": "application/json" },
        signal: productSessionSignal(), body: JSON.stringify(input),
      });
      const result = await response.json() as { status?: string; error?: { code?: string } };
      if (!live()) return false;
      if (response.ok && result.status === "committed") {
        sessionStorage.removeItem(journalKey);
        setPending(undefined); setError(""); setLabel("");
        if (input.change.type === "create") setAdding(false);
        if (input.change.type === "rename") {
          const id = input.change.id;
          setNames((old) => { const next = { ...old }; delete next[id]; return next; });
        }
        await load();
        return true;
      } else if (response.status === 409 && result.error?.code === "revision_conflict") {
        sessionStorage.removeItem(journalKey); setPending(undefined);
        setError("Subscriptions changed elsewhere. Review the refreshed list before applying your change again.");
        await load();
      } else if (subscriptionWriteRejected(response.status, result.error?.code)) {
        sessionStorage.removeItem(journalKey); setPending(undefined);
        setError(subscriptionRejectionText(result.error!.code!));
        await load();
      } else {
        setError("The change is not confirmed. Retry uses the same operation; no login or account rotation will run.");
      }
    } catch { if (live()) setError("The change is not confirmed. Retry uses the same operation."); }
    finally { lock.current = false; if (live()) setBusy(false); }
    return false;
  }
  function save(change: Change): Promise<boolean> {
    if (!snapshot || pending || busy || !journalReady) return Promise.resolve(false);
    return commit({ operation: crypto.randomUUID(), dataset: snapshot.dataset.dataset_id,
      expected_revision: snapshot.registry.revision, change });
  }
  const disabled = busy || Boolean(pending) || !snapshot || !journalReady;
  const named: SubscriptionCards["named"] = snapshot?.registry.accounts.filter((account) => !snapshot.revoked_accounts?.includes(account.id)).map((account) => {
    const entry = providers.find((entry) => entry.provider_id === account.provider_id);
    const control = accountControl(snapshot, account.id);
    const providerLabel = entry?.manifest.display.name ?? account.provider_id;
    return { id: account.id, provider: account.provider_id,
    content: <Paper key={`${owner}:${snapshot.dataset.dataset_id}:${account.id}`} variant="outlined" sx={providerAccountCardSx} data-subscription-account={account.id}>
      <Stack spacing={1}>
        <ProviderAccountIdentity manifests={entry ? [entry.manifest] : []} title={control ? `${providerLabel} · ${account.label}` : account.label}
          summary={`${providerLabel}${snapshot.registry.defaults[account.provider_id] === account.id ? " · Default" : ""}${account.revoking ? " · Removal requested" : !account.enabled ? " · Disabled" : ""}`}
          actions={<Button size="small" aria-expanded={editing.includes(account.id)} onClick={() => setEditing((old) => old.includes(account.id) ? old.filter((id) => id !== account.id) : [...old, account.id])}>{editing.includes(account.id) ? "Done" : "Manage"}</Button>}>
          {owner && (snapshot.portable_providers?.includes(account.provider_id)
            ? <SubscriptionApiKey account={account.id} credential={snapshot.portable_credentials?.find((item) => item.account_id === account.id)}
              method={entry?.manifest.authentication.methods.find((method) => method.flow === "secret_input")}
              expanded={editing.includes(account.id)} disabled={disabled || !account.enabled || Boolean(account.revoking)} onChange={() => void load()} />
            : <SubscriptionLogins scope={{ owner, dataset: snapshot.dataset.dataset_id, account: account.id, provider: account.provider_id }} label={account.label}
              vault={snapshot.subscription_providers?.includes(account.provider_id) ? snapshot.portable_credentials?.find((item) => item.account_id === account.id) ?? null : undefined}
              onSignedIn={() => void load()}
              expanded={editing.includes(account.id)} disabled={disabled || !account.enabled || Boolean(account.revoking)} />)}
          {control && <AccountControlsPanel control={control} disabled={disabled || Boolean(account.revoking)} onChange={() => void load()}
            observation={native.accounts.find((item) => item.account_id === account.id && item.provider_id === account.provider_id)} />}
        </ProviderAccountIdentity>
        {editing.includes(account.id) && <Stack spacing={1}>
          <Stack direction="row" spacing={1}>
            <TextField size="small" label="Account label" value={names[account.id] ?? account.label} disabled={disabled} fullWidth sx={{ minWidth: 0 }} onChange={(event) => setNames((old) => ({ ...old, [account.id]: event.target.value }))} />
            <Button disabled={disabled || !validSubscriptionLabel(names[account.id] ?? "") || names[account.id]?.trim() === account.label} onClick={() => save({ type: "rename", id: account.id, label: names[account.id]!.trim() })}>Save</Button>
          </Stack>
          <Stack direction="row" spacing={0.5} flexWrap="wrap" useFlexGap>
            <Button disabled={disabled || account.revoking} onClick={() => save({ type: "set_enabled", id: account.id, enabled: !account.enabled })}>{account.revoking ? "Revoked access" : account.enabled ? "Disable" : "Enable"}</Button>
            {!control && snapshot.registry.defaults[account.provider_id] !== account.id &&
              <Button disabled={disabled || !account.enabled || !providerSubscriptionEnabled(snapshot, account.provider_id)} onClick={() => save({ type: "set_default", provider_id: account.provider_id, id: account.id })}>Use by default</Button>}
            {/* Clears each default naming it (an adopted shared-scope account is the default of every member), so it can be disabled. */}
            {!control && accountDefaults(snapshot, account).slice(0, 1).map((provider) =>
              <Button key={provider} disabled={disabled} onClick={() => save({ type: "set_default", provider_id: provider, id: "default" })}>
                {provider === account.provider_id && accountDefaults(snapshot, account).length === 1 ? "Stop using by default" : `Stop using by default (${provider})`}
              </Button>)}
            {owner && <SubscriptionBrowserActions account={account} owner={owner} dataset={snapshot.dataset.dataset_id} disabled={disabled} shared={sharedBrowser(snapshot, account.id)}
              onSetBrowserEnabled={(enabled) => save({ type: "set_browser_enabled", id: account.id, enabled })} />}
            {owner && <SubscriptionRevocation account={account.id} label={account.label} revoking={Boolean(account.revoking)} owner={owner}
              dataset={snapshot.dataset.dataset_id} revision={snapshot.registry.revision} disabled={disabled} onChange={() => void load()} />}
          </Stack>
          <Typography variant="caption" color="text.secondary">Disabling blocks access in Cowboy but does not stop running processes or sign out. Re-enabling requires a new login.</Typography>
        </Stack>}
      </Stack>
    </Paper>,
  }; }) ?? [];
  return <Stack spacing={1.5}>
    <Stack direction="row" alignItems="center" justifyContent="space-between" spacing={1}>
      <Typography variant="subtitle1" color="text.primary">Accounts</Typography>
      <Button aria-expanded={adding} onClick={() => setAdding((old) => !old)}>{adding ? "Close add account" : "Add account"}</Button>
    </Stack>
    {error && <Alert severity="warning" action={<Button disabled={busy || (Boolean(pending) && (!snapshot || !journalReady))} onClick={() => pending ? void commit(pending) : void load()}>Retry</Button>}>{error}</Alert>}
    {adding && <Stack spacing={1}>
      <Stack direction={{ xs: "column", sm: "row" }} spacing={1}>
      <TextField select label="Provider" value={provider} onChange={(event) => setProvider(event.target.value)} disabled={disabled} fullWidth size="small">
        {providers.map((entry) => <MenuItem key={entry.provider_id} value={entry.provider_id}>{entry.manifest.display.name}</MenuItem>)}
      </TextField>
      <TextField label="Account label" value={label} onChange={(event) => setLabel(event.target.value)} disabled={disabled} fullWidth size="small" />
      <Button disabled={disabled || !provider || !validSubscriptionLabel(label)} onClick={() => save({ type: "create", provider_id: provider, label: label.trim() })}>Add</Button>
      </Stack>
      <Typography variant="caption" color="text.secondary">Add an account, then choose a Machine to see its supported sign-in methods.</Typography>
    </Stack>}
    {snapshot?.session_admission !== "enabled" && <Typography variant="caption" color="text.secondary">Additional accounts support sign-in setup. Creating sessions with them is not enabled yet.</Typography>}
    {renderAccounts
      ? renderAccounts({ named, namedOnly: [...snapshot?.named_only_providers ?? [], ...snapshot?.retired_default_providers ?? []] })
      : named.map((card) => card.content)}
    <BrowserShareSettings snapshot={snapshot} disabled={disabled} save={save} />
  </Stack>;
}
