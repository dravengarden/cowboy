import { useEffect, useRef, useSyncExternalStore } from "react";
import { PREFERENCES_CHANGED } from "./aboutPreferences";
import { productSessionSignal } from "./productSessionEnd";
import { productSyncPrincipal } from "./productSyncIdentity";
import { type AccountControls, validAccountControls } from "./accountControls";

export interface SubscriptionAccount {
  id: string;
  provider_id: string;
  label: string;
  enabled: boolean;
  revoking?: boolean;
  /** Owner-enabled Machine browser for this account (`set_browser_enabled`). */
  browser_enabled?: boolean;
}
/** A browser identity that may join a shared browser profile. */
export type BrowserShareMemberRef = { account_id: string };
export interface BrowserShareMember {
  member: BrowserShareMemberRef;
  /** Its account-usage family; a share holds at most one per family. */
  family: string;
  /** The member's own browser id (its account id). */
  account_id: string;
}
export interface BrowserShareFamilies {
  version: number;
  providers: Record<string, string>;
}
/** An owner-defined shared browser profile (docs/machine-browser.md,
 * "Shared browser profiles"). */
export interface BrowserShare {
  share_id: string;
  label: string;
  created_ms: number;
  revision: number;
  /** Deleted; its profile is still being revoked on its Machine. */
  deleting: boolean;
  members: BrowserShareMember[];
  /** The member each family of the group's profile belongs to, kept after
   * it leaves until the profile is reset: no other account of that family
   * may join meanwhile. */
  bound?: { family: string; account_id: string }[];
}
/** Public key state of a portable account (an API key held by the Service). */
export interface PortableCredential {
  account_id: string;
  provider_id: string;
  authentication_state: string;
  auth_generation: number;
  distribution_state: string;
  updated_at_ms: number;
}
export interface SubscriptionSnapshot {
  dataset: { dataset_id: string; user_id: string };
  registry: { revision: number; accounts: SubscriptionAccount[]; defaults: Record<string, string> };
  /** Accounts whose permanent revocation finished; they leave account lists. */
  revoked_accounts?: string[];
  session_admission: string;
  session_providers?: string[];
  /** The Service gate of the optional Machine browser (`accounts.machine_browser`). */
  machine_browser?: "enabled" | "not_enabled";
  browser_shares?: BrowserShare[];
  /** The Service's static family table (`browser_families`): Provider id
   * → family. Nothing else may ever share a browser. */
  browser_share_families?: BrowserShareFamilies;
  /** Providers whose accounts take an API key instead of a sign-in. */
  portable_providers?: string[];
  /** Credential scopes: one portable account serves every listed Provider. */
  portable_scopes?: string[][];
  portable_credentials?: PortableCredential[];
  /** Subscription Providers: one sign-in on any Machine lands in the
   * account's Service vault and serves every Machine. */
  subscription_providers?: string[];
  /** Providers whose sessions need a named account (Antigravity). */
  named_only_providers?: string[];
  /** Providers whose legacy default card is retired: no Service account
   * (docs/legacy-card-retirement.md). */
  retired_default_providers?: string[];
  /** Their accounts' admission, state, active turn, fences, uncertain
   * effects and logout (`accounts.list().account_controls`). */
  account_controls?: AccountControls;
}

type PortableScopes = { portable_scopes?: string[][] } | undefined;
/** Whether an account of `accountProvider` serves sessions of `provider`
 * (the same Provider, or a portable credential scope that holds both). */
export function accountServes(data: PortableScopes, accountProvider: string, provider: string): boolean {
  return accountProvider === provider ||
    (data?.portable_scopes ?? []).some((scope) => scope.includes(provider) && scope.includes(accountProvider));
}
/** The default account id the Service resolves for `provider`: its own
 * default, else the first default among its portable scope (Provider order). */
export function providerDefault(data: PortableScopes & { registry: { defaults: Record<string, string> } } | undefined, provider: string): string | undefined {
  if (!data) return undefined;
  const own = data.registry.defaults[provider];
  if (own) return own;
  const scope = (data.portable_scopes ?? []).find((members) => members.includes(provider)) ?? [];
  return [...scope].sort().map((member) => data.registry.defaults[member]).find(Boolean);
}

/** The Providers whose default is `account`, its own Provider first. An
 * adopted shared-scope account (DeepSeek) is the default of every member;
 * each must be cleared before the account can be disabled. */
export function accountDefaults(snapshot: { registry: { defaults: Record<string, string> } }, account: { id: string; provider_id: string }): string[] {
  return Object.entries(snapshot.registry.defaults).filter(([, id]) => id === account.id).map(([provider]) => provider)
    .sort((left, right) => Number(right === account.provider_id) - Number(left === account.provider_id) || left.localeCompare(right));
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
function text(value: unknown): value is string { return typeof value === "string" && value.length > 0; }
function validShareMember(value: unknown): value is BrowserShareMember {
  if (!record(value) || !record(value.member) || !text(value.family) || !text(value.account_id)) return false;
  const keys = Object.keys(value.member);
  // A Controller from before the legacy card retirement (Web deployed
  // first, or a rollback) still reports `{card}` members; they are read and
  // shown as bound families (`withoutLegacyShareMembers`), never edited.
  return keys.length === 1 && (text(value.member.account_id) || text(value.member.card));
}

/** The snapshot with every legacy `{card}` share member turned into a bound
 * family of its share: the card's cookies keep that family out of the
 * profile, and writes stay named-only. */
export function withoutLegacyShareMembers(snapshot: SubscriptionSnapshot): SubscriptionSnapshot {
  const legacy = (row: BrowserShareMember) => !text((row.member as Record<string, unknown>).account_id);
  if (!snapshot.browser_shares?.some((share) => share.members.some(legacy))) return snapshot;
  return {
    ...snapshot,
    browser_shares: snapshot.browser_shares.map((share) => ({
      ...share,
      members: share.members.filter((row) => !legacy(row)),
      bound: [...share.bound ?? [], ...share.members.filter(legacy)
        .filter((row) => !share.bound?.some((bound) => bound.family === row.family))
        .map((row) => ({ family: row.family, account_id: row.account_id }))],
    })),
  };
}

/** A Machine browser of a legacy card that a Controller from before the
 * retirement still advertises (`legacy_browsers`), read-only. */
export interface LegacyCardBrowser { account_id: string; providers: string[]; available: boolean }
export function legacyCardBrowsers(snapshot: SubscriptionSnapshot): LegacyCardBrowser[] {
  const value = (snapshot as { legacy_browsers?: unknown }).legacy_browsers;
  return Array.isArray(value)
    ? value.filter((row): row is LegacyCardBrowser => record(row) && text(row.account_id) &&
      Array.isArray(row.providers) && row.providers.every(text) && typeof row.available === "boolean")
    : [];
}

/** What a session of a retired card's `default` account offers instead of a
 * Service sign-in (the Controller's refusal reads the same). */
export const RETIRED_SESSION_GUIDANCE = "This session ran on a retired legacy default account; continue it in a new agent on a named account.";
/** The account-usage families of the retired legacy default cards. */
const RETIRED_CARD_FAMILIES = ["anthropic", "deepseek", "openai", "xai"];
/** Whether `provider`'s Service (`default`) account was a retired legacy
 * card. An older Controller does not report `retired_default_providers`, so
 * its family table decides as well. */
export function retiredDefaultProvider(snapshot: SubscriptionSnapshot | undefined, provider: string): boolean {
  if (!snapshot) return false;
  if (snapshot.retired_default_providers?.includes(provider)) return true;
  const family = snapshot.browser_share_families?.providers[provider];
  return family !== undefined && RETIRED_CARD_FAMILIES.includes(family);
}
function validShareFamilies(value: unknown): value is BrowserShareFamilies {
  return record(value) && typeof value.version === "number" && record(value.providers) &&
    Object.values(value.providers).every(text);
}
function validBrowserShare(value: unknown): value is BrowserShare {
  return record(value) && text(value.share_id) && typeof value.label === "string" &&
    typeof value.created_ms === "number" && typeof value.revision === "number" &&
    typeof value.deleting === "boolean" && Array.isArray(value.members) && value.members.every(validShareMember) &&
    (value.bound === undefined || (Array.isArray(value.bound) &&
      value.bound.every((row: unknown) => record(row) && text(row.family) && text(row.account_id))));
}

function validPortableCredential(value: unknown): value is PortableCredential {
  return record(value) && text(value.account_id) && text(value.provider_id) && text(value.authentication_state) &&
    typeof value.auth_generation === "number" && text(value.distribution_state) && typeof value.updated_at_ms === "number";
}

export function validSubscriptionSnapshot(value: unknown, owner: string | undefined): value is SubscriptionSnapshot {
  if (!owner || !record(value) || !record(value.dataset) || !record(value.registry)) return false;
  const { dataset, registry } = value;
  return dataset.user_id === owner && text(dataset.dataset_id) &&
    typeof registry.revision === "number" && Number.isSafeInteger(registry.revision) && registry.revision >= 0 &&
    (value.session_providers === undefined || (Array.isArray(value.session_providers) && value.session_providers.every(text))) &&
    (value.revoked_accounts === undefined || (Array.isArray(value.revoked_accounts) && value.revoked_accounts.every(text))) &&
    (value.machine_browser === undefined || value.machine_browser === "enabled" || value.machine_browser === "not_enabled") &&
    (value.browser_shares === undefined || (Array.isArray(value.browser_shares) && value.browser_shares.every(validBrowserShare))) &&
    (value.browser_share_families === undefined || validShareFamilies(value.browser_share_families)) &&
    (value.portable_providers === undefined || (Array.isArray(value.portable_providers) && value.portable_providers.every(text))) &&
    (value.portable_scopes === undefined || (Array.isArray(value.portable_scopes) &&
      value.portable_scopes.every((scope: unknown) => Array.isArray(scope) && scope.every(text)))) &&
    (value.portable_credentials === undefined || (Array.isArray(value.portable_credentials) && value.portable_credentials.every(validPortableCredential))) &&
    (value.subscription_providers === undefined || (Array.isArray(value.subscription_providers) && value.subscription_providers.every(text))) &&
    (value.retired_default_providers === undefined || (Array.isArray(value.retired_default_providers) && value.retired_default_providers.every(text))) &&
    (value.named_only_providers === undefined || (Array.isArray(value.named_only_providers) && value.named_only_providers.every(text))) &&
    (value.account_controls === undefined || validAccountControls(value.account_controls)) &&
    typeof value.session_admission === "string" && record(registry.defaults) && Object.values(registry.defaults).every(text) &&
    Array.isArray(registry.accounts) && registry.accounts.every((account: unknown) => record(account) &&
      text(account.id) && text(account.provider_id) && text(account.label) && typeof account.enabled === "boolean" &&
      (account.revoking === undefined || typeof account.revoking === "boolean") &&
      (account.browser_enabled === undefined || typeof account.browser_enabled === "boolean"));
}

const empty: { data?: SubscriptionSnapshot; error: string } = { error: "" };
let snapshot = empty;
let snapshotOwner: string | undefined;
let pending: Promise<void> | undefined;
let invalidated = false;
const listeners = new Set<() => void>();
function current() {
  return !productSessionSignal().aborted && snapshotOwner === productSyncPrincipal() ? snapshot : empty;
}
function publish() { for (const listener of listeners) listener(); }

/** A single inventory request for the whole UI, including many session/call badges.
 * A preference event during a request schedules another read instead of losing it. */
async function refresh(): Promise<void> {
  if (productSessionSignal().aborted || !productSyncPrincipal()) return;
  if (pending) return pending;
  pending = (async () => {
    do {
      invalidated = false;
      const owner = productSyncPrincipal();
      const signal = productSessionSignal();
      try {
        const response = await fetch("/api/provider-accounts", { cache: "no-store", signal });
        if (!response.ok) throw new Error("Could not load registered accounts.");
        const value: unknown = await response.json();
        if (signal.aborted || owner !== productSyncPrincipal()) return;
        if (!validSubscriptionSnapshot(value, owner)) throw new Error("Account inventory changed. Reload to continue.");
        const old = current().data;
        if (!old || old.dataset.dataset_id !== value.dataset.dataset_id ||
          value.registry.revision >= old.registry.revision
        ) { snapshot = { data: withoutLegacyShareMembers(value), error: "" }; snapshotOwner = owner; }
      } catch (cause) {
        if (!signal.aborted && owner === productSyncPrincipal()) {
          snapshot = { ...current(), error: cause instanceof Error ? cause.message : "Could not load registered accounts." };
          snapshotOwner = owner;
        }
      }
      publish();
    } while (invalidated && !productSessionSignal().aborted);
  })().finally(() => {
    pending = undefined;
    // A preference event can run after the loop's final check but before this
    // continuation. Keep it in the same refresh promise instead of losing it.
    if (invalidated && listeners.size > 0 && !productSessionSignal().aborted) return refresh();
    return undefined;
  });
  return pending;
}
function reload(): Promise<void> { invalidated = true; return refresh(); }
function changed(event: Event) {
  const scopes = (event as CustomEvent<Record<string, number> | undefined>).detail;
  if (!scopes || Object.hasOwn(scopes, "provider_accounts") || Object.hasOwn(scopes, "subscriptions")) {
    invalidated = true;
    void refresh();
  }
}
function subscribe(listener: () => void) {
  if (listeners.size === 0) {
    window.addEventListener(PREFERENCES_CHANGED, changed);
    productSessionSignal().addEventListener("abort", clear, { once: true });
  }
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0) {
      invalidated = true;
      window.removeEventListener(PREFERENCES_CHANGED, changed);
      productSessionSignal().removeEventListener("abort", clear);
    }
  };
}
function clear() { snapshot = empty; snapshotOwner = undefined; publish(); }
/** Browser fixtures share one page: start a surface from a first load. */
export function resetSubscriptionAccountsForTest(): void { invalidated = false; clear(); }

/** The shared inventory already loaded on this page, without a request. */
export function peekSubscriptionAccounts(): SubscriptionSnapshot | undefined {
  return current().data;
}

/** Read-only account inventory; mutations continue through the shared writer. */
export function useSubscriptionAccounts(connected = true) {
  const value = useSyncExternalStore(subscribe, current, () => empty);
  const previous = useRef(connected);
  useEffect(() => {
    if (!current().data || invalidated || (connected && !previous.current)) void refresh();
    previous.current = connected;
  }, [connected]);
  return { ...value, refresh: reload };
}
