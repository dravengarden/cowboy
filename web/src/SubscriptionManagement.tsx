import { Alert, Box, Button, MenuItem, Paper, Stack, TextField, Typography } from "@mui/material";
import type { ReactNode } from "react";
import { useEffect, useRef, useState } from "react";
import { latestProviderEntries, useProviderCatalog } from "./providerCatalog";
import { productSessionSignal } from "./productSessionEnd";
import { productSyncPrincipal } from "./productSyncIdentity";
import { PREFERENCES_CHANGED } from "./aboutPreferences";
import { providerSubscriptionEnabled } from "./SessionSubscription";
import { SubscriptionLogins } from "./SubscriptionLogins";
import { SubscriptionRevocation } from "./SubscriptionRevocation";
import { ProviderAccountIdentity, providerAccountCardSx } from "./ProviderAccountIdentity";
import { legacySubscriptionLabel, type SubscriptionDisplay, validLegacyLabels, validSubscriptionDisplay, validSubscriptionLabel } from "./subscriptionDisplay";
export { validSubscriptionLabel } from "./subscriptionDisplay";

interface Account { id: string; provider_id: string; label: string; enabled: boolean; revoking?: boolean }
interface Snapshot {
  dataset: { dataset_id: string; user_id: string };
  registry: { revision: number; accounts: Account[]; defaults: Record<string, string> };
  display_preferences?: SubscriptionDisplay;
  session_admission: string;
  session_providers?: string[];
}
type Change = { type: "create"; provider_id: string; label: string }
  | { type: "rename"; id: string; label: string }
  | { type: "set_enabled"; id: string; enabled: boolean }
  | { type: "set_default"; provider_id: string; id: string }
  | { type: "set_legacy_labels"; labels: Record<string, string> };
interface Pending { operation: string; dataset: string; expected_revision: number; change: Change }

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
function revision(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}
function text(value: unknown): value is string { return typeof value === "string" && value.length > 0; }

export function validSubscriptionSnapshot(value: unknown, owner: string | undefined): value is Snapshot {
  if (!owner || !record(value) || !record(value.dataset) || !record(value.registry)) return false;
  const { dataset, registry } = value;
  return dataset.user_id === owner && text(dataset.dataset_id) && revision(registry.revision) &&
    (value.display_preferences === undefined || validSubscriptionDisplay(value.display_preferences)) &&
    (value.session_providers === undefined || (Array.isArray(value.session_providers) && value.session_providers.every(text))) &&
    typeof value.session_admission === "string" && record(registry.defaults) && Object.values(registry.defaults).every(text) &&
    Array.isArray(registry.accounts) && registry.accounts.every((account: unknown) => record(account) &&
      text(account.id) && text(account.provider_id) && text(account.label) && typeof account.enabled === "boolean" &&
      (account.revoking === undefined || typeof account.revoking === "boolean"));
}

export function validPendingSubscription(value: unknown): value is Pending {
  if (!record(value) || !text(value.operation) || !text(value.dataset) || !revision(value.expected_revision) || !record(value.change)) return false;
  if (Object.keys(value).some((key) => !["operation", "dataset", "expected_revision", "change"].includes(key))) return false;
  const change = value.change;
  const fields = change.type === "create" ? ["type", "provider_id", "label"]
    : change.type === "set_legacy_labels" ? ["type", "labels"]
    : change.type === "rename" ? ["type", "id", "label"]
    : change.type === "set_enabled" ? ["type", "id", "enabled"]
    : change.type === "set_default" ? ["type", "provider_id", "id"] : [];
  if (fields.length === 0 || Object.keys(change).some((key) => !fields.includes(key))) return false;
  switch (change.type) {
    case "set_legacy_labels": return validLegacyLabels(change.labels);
    case "create": return text(change.provider_id) && typeof change.label === "string" && validSubscriptionLabel(change.label);
    case "rename": return text(change.id) && typeof change.label === "string" && validSubscriptionLabel(change.label);
    case "set_enabled": return text(change.id) && typeof change.enabled === "boolean";
    case "set_default": return text(change.provider_id) && text(change.id);
    default: return false;
  }
}

/** Only explicit domain rejections establish that this mutation did not commit. */
export function subscriptionWriteRejected(status: number, code: string | undefined): boolean {
  return status === 422 && ["invalid_account", "account_capacity", "account_is_default", "invalid_provider", "account_unavailable", "invalid_preferences"].includes(code ?? "");
}

export interface SubscriptionCards {
  legacy: (provider: string) => { title: string | undefined; controls: ReactNode };
  named: { id: string; provider: string; content: ReactNode }[];
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
        (value.registry.revision >= old.registry.revision && (value.display_preferences?.revision ?? 0) >= (old.display_preferences?.revision ?? 0)) ? value : old);
    } catch (cause) { if (live() && sequence === loading.current) setError(String(cause)); }
  }
  useEffect(() => {
    alive.current = true;
    try {
      const saved = sessionStorage.getItem(journalKey);
      if (saved) {
        const value: unknown = JSON.parse(saved);
        if (!validPendingSubscription(value)) throw new Error("Invalid pending subscription change.");
        setPending(value);
        setError("A previous change needs reconciliation. Retry sends the same operation.");
      }
      setJournalReady(true);
    } catch { setError("Could not read the pending subscription change. Local storage must be available before editing."); }
    void load();
    const refresh = (event: Event) => {
      const scopes = (event as CustomEvent<Record<string, number> | undefined>).detail;
      if (!scopes || Object.hasOwn(scopes, "provider_accounts") || Object.hasOwn(scopes, "subscriptions")) void load();
    };
    window.addEventListener(PREFERENCES_CHANGED, refresh);
    return () => { alive.current = false; ++loading.current; window.removeEventListener(PREFERENCES_CHANGED, refresh); };
    // This panel belongs to one authenticated product mount.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  async function commit(input: Pending) {
    if (lock.current || !snapshot || !journalReady || !live()) return;
    if (input.dataset !== snapshot.dataset.dataset_id) {
      setError("This pending change belongs to another dataset. It will not be submitted.");
      return;
    }
    lock.current = true;
    setBusy(true);
    try {
      // Persist before sending so remounting or reloading this tab cannot silently
      // replace an uncertain create with a second operation.
      sessionStorage.setItem(journalKey, JSON.stringify(input));
      setPending(input);
      const legacy = input.change.type === "set_legacy_labels";
      const body = input.change.type === "set_legacy_labels"
        ? { operation: input.operation, dataset: input.dataset, scope: "subscriptions", expected_revision: input.expected_revision,
          changes: { legacy_labels: input.change.labels } }
        : input;
      const response = await fetch(legacy ? "/api/preferences" : "/api/provider-accounts", {
        method: "PUT", headers: { "Content-Type": "application/json" },
        signal: productSessionSignal(), body: JSON.stringify(body),
      });
      const result = await response.json() as { status?: string; error?: { code?: string } };
      if (!live()) return;
      if (response.ok && result.status === "committed") {
        sessionStorage.removeItem(journalKey);
        setPending(undefined); setError(""); setLabel("");
        if (input.change.type === "create") setAdding(false);
        if (input.change.type === "rename") {
          const id = input.change.id;
          setNames((old) => { const next = { ...old }; delete next[id]; return next; });
        }
        if (input.change.type === "set_legacy_labels") {
          const saved = input.change.labels;
          setNames((old) => Object.fromEntries(Object.entries(old).filter(([id, name]) =>
            !id.startsWith("legacy:") || name.trim() !== saved[id.slice("legacy:".length)]
          )));
        }
        await load();
      } else if (response.status === 409 && result.error?.code === "revision_conflict") {
        sessionStorage.removeItem(journalKey); setPending(undefined);
        setError("Subscriptions changed elsewhere. Review the refreshed list before applying your change again.");
        await load();
      } else if (subscriptionWriteRejected(response.status, result.error?.code)) {
        sessionStorage.removeItem(journalKey); setPending(undefined);
        setError(`Change rejected (${result.error!.code}). Review the list before trying again.`);
        await load();
      } else {
        setError("The change is not confirmed. Retry uses the same operation; no login or account rotation will run.");
      }
    } catch { if (live()) setError("The change is not confirmed. Retry uses the same operation."); }
    finally { lock.current = false; if (live()) setBusy(false); }
  }
  function save(change: Change) {
    if (!snapshot || pending || busy || !journalReady) return;
    if (change.type === "set_legacy_labels" && !snapshot.display_preferences) return;
    void commit({ operation: crypto.randomUUID(), dataset: snapshot.dataset.dataset_id,
      expected_revision: change.type === "set_legacy_labels" ? snapshot.display_preferences!.revision : snapshot.registry.revision, change });
  }
  const legacyProviders = snapshot?.display_preferences ? [...new Set([
    ...providers.map((entry) => entry.provider_id), ...snapshot.registry.accounts.map((account) => account.provider_id),
    ...Object.keys(snapshot.display_preferences.values.legacy_labels),
  ])] : [];
  const disabled = busy || Boolean(pending) || !snapshot || !journalReady;
  const legacy: SubscriptionCards["legacy"] = (id) => {
    const current = legacySubscriptionLabel(snapshot?.display_preferences, id);
    const draft = names[`legacy:${id}`];
    return { title: snapshot?.display_preferences?.values.legacy_labels[id], controls: <Stack spacing={1}>
      <Stack direction="row" spacing={1} alignItems="center">
        <TextField size="small" label="Account label" value={draft ?? current} disabled={disabled || !snapshot?.display_preferences} fullWidth sx={{ minWidth: 0 }}
          onChange={(event) => setNames((old) => ({ ...old, [`legacy:${id}`]: event.target.value }))} />
        <Button disabled={disabled || !snapshot?.display_preferences || !validSubscriptionLabel(draft ?? "") || draft?.trim() === current}
          onClick={() => save({ type: "set_legacy_labels", labels: { ...snapshot!.display_preferences!.values.legacy_labels, [id]: draft!.trim() } })}>Save</Button>
      </Stack>
      <Typography variant="caption" color="text.secondary">{id} · Service account{!snapshot?.registry.defaults[id] ? " · Default" : ""}</Typography>
      {snapshot?.registry.defaults[id] && <Button disabled={disabled} onClick={() => save({ type: "set_default", provider_id: id, id: "default" })}>Use by default</Button>}
    </Stack> };
  };
  const named: SubscriptionCards["named"] = snapshot?.registry.accounts.map((account) => {
    const entry = providers.find((entry) => entry.provider_id === account.provider_id);
    return { id: account.id, provider: account.provider_id,
    content: <Paper key={`${owner}:${snapshot.dataset.dataset_id}:${account.id}`} variant="outlined" sx={providerAccountCardSx} data-subscription-account={account.id}>
      <Stack spacing={1}>
        <ProviderAccountIdentity manifests={entry ? [entry.manifest] : []} title={account.label}
          summary={`${entry?.manifest.display.name ?? account.provider_id}${snapshot.registry.defaults[account.provider_id] === account.id ? " · Default" : ""}${account.revoking ? " · Removal requested" : !account.enabled ? " · Disabled" : ""}`}
          actions={<Button size="small" aria-expanded={editing.includes(account.id)} onClick={() => setEditing((old) => old.includes(account.id) ? old.filter((id) => id !== account.id) : [...old, account.id])}>{editing.includes(account.id) ? "Done" : "Manage"}</Button>}>
          {owner && <SubscriptionLogins scope={{ owner, dataset: snapshot.dataset.dataset_id, account: account.id, provider: account.provider_id }} label={account.label}
            expanded={editing.includes(account.id)} disabled={disabled || !account.enabled || Boolean(account.revoking)} />}
        </ProviderAccountIdentity>
        {editing.includes(account.id) && <Stack spacing={1}>
          <Stack direction="row" spacing={1}>
            <TextField size="small" label="Account label" value={names[account.id] ?? account.label} disabled={disabled} fullWidth sx={{ minWidth: 0 }} onChange={(event) => setNames((old) => ({ ...old, [account.id]: event.target.value }))} />
            <Button disabled={disabled || !validSubscriptionLabel(names[account.id] ?? "") || names[account.id]?.trim() === account.label} onClick={() => save({ type: "rename", id: account.id, label: names[account.id]!.trim() })}>Save</Button>
          </Stack>
          <Stack direction="row" spacing={0.5} flexWrap="wrap" useFlexGap>
            <Button disabled={disabled || account.revoking} onClick={() => save({ type: "set_enabled", id: account.id, enabled: !account.enabled })}>{account.revoking ? "Revoked access" : account.enabled ? "Disable" : "Enable"}</Button>
            <Button disabled={disabled || !account.enabled || snapshot.registry.defaults[account.provider_id] === account.id || !providerSubscriptionEnabled(snapshot, account.provider_id)} onClick={() => save({ type: "set_default", provider_id: account.provider_id, id: account.id })}>Use by default</Button>
            {snapshot.registry.defaults[account.provider_id] === account.id && <Button disabled={disabled} onClick={() => save({ type: "set_default", provider_id: account.provider_id, id: "default" })}>Use existing account by default</Button>}
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
    {renderAccounts ? renderAccounts({ legacy, named }) : <>
      {legacyProviders.map((id) => <Box key={`legacy:${id}`}>{legacy(id).controls}</Box>)}
      {named.map((card) => card.content)}
    </>}
  </Stack>;
}
