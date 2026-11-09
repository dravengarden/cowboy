import { useEffect, useRef, useSyncExternalStore } from "react";
import { PREFERENCES_CHANGED } from "./aboutPreferences";
import { productSessionSignal } from "./productSessionEnd";
import { productSyncPrincipal } from "./productSyncIdentity";
import { type SubscriptionDisplay, validSubscriptionDisplay } from "./subscriptionDisplay";

export interface SubscriptionAccount {
  id: string;
  provider_id: string;
  label: string;
  enabled: boolean;
  revoking?: boolean;
  /** Owner-enabled Machine browser for this account (`set_browser_enabled`). */
  browser_enabled?: boolean;
}
/** The Machine browser of a legacy default usage card (the Service-collected
 * default account of an account-usage provider such as `anthropic` or
 * `deepseek`): its own browser key, never shared with another card or a
 * named account, and its owner enablement (off by default). */
export interface LegacyBrowser {
  /** The usage card id: its account-usage provider. */
  card: string;
  /** The card's own browser key for the browser routes. */
  account_id: string;
  /** The Providers whose default account the card shows. */
  providers: string[];
  browser_enabled: boolean;
  /** The Service collects the card now. */
  available: boolean;
  /** The shared browser profile the card uses, if any. */
  share_id?: string | null;
}
/** A browser identity that may join a shared browser profile. */
export type BrowserShareMemberRef = { account_id: string } | { card: string };
export interface BrowserShareMember {
  member: BrowserShareMemberRef;
  /** Its account-usage family; a share holds at most one per family. */
  family: string;
  /** The member's own browser id (named account id or card key). */
  account_id: string;
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
export interface SubscriptionSnapshot {
  dataset: { dataset_id: string; user_id: string };
  registry: { revision: number; accounts: SubscriptionAccount[]; defaults: Record<string, string> };
  display_preferences?: SubscriptionDisplay;
  /** Accounts whose permanent revocation finished; they leave account lists. */
  revoked_accounts?: string[];
  session_admission: string;
  session_providers?: string[];
  /** The Service gate of the optional Machine browser (`accounts.machine_browser`). */
  machine_browser?: "enabled" | "not_enabled";
  legacy_browsers?: LegacyBrowser[];
  browser_shares?: BrowserShare[];
  /** Provider id → account-usage family. */
  browser_share_families?: Record<string, string>;
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
function text(value: unknown): value is string { return typeof value === "string" && value.length > 0; }
function validLegacyBrowser(value: unknown): value is LegacyBrowser {
  return record(value) && text(value.card) && text(value.account_id) && Array.isArray(value.providers) &&
    value.providers.every(text) && typeof value.browser_enabled === "boolean" && typeof value.available === "boolean" &&
    (value.share_id === undefined || value.share_id === null || text(value.share_id));
}
function validShareMember(value: unknown): value is BrowserShareMember {
  if (!record(value) || !record(value.member) || !text(value.family) || !text(value.account_id)) return false;
  const keys = Object.keys(value.member);
  return keys.length === 1 && (text(value.member.account_id) || text(value.member.card));
}
function validBrowserShare(value: unknown): value is BrowserShare {
  return record(value) && text(value.share_id) && typeof value.label === "string" &&
    typeof value.created_ms === "number" && typeof value.revision === "number" &&
    typeof value.deleting === "boolean" && Array.isArray(value.members) && value.members.every(validShareMember) &&
    (value.bound === undefined || (Array.isArray(value.bound) &&
      value.bound.every((row: unknown) => record(row) && text(row.family) && text(row.account_id))));
}

export function validSubscriptionSnapshot(value: unknown, owner: string | undefined): value is SubscriptionSnapshot {
  if (!owner || !record(value) || !record(value.dataset) || !record(value.registry)) return false;
  const { dataset, registry } = value;
  return dataset.user_id === owner && text(dataset.dataset_id) &&
    typeof registry.revision === "number" && Number.isSafeInteger(registry.revision) && registry.revision >= 0 &&
    (value.display_preferences === undefined || validSubscriptionDisplay(value.display_preferences)) &&
    (value.session_providers === undefined || (Array.isArray(value.session_providers) && value.session_providers.every(text))) &&
    (value.revoked_accounts === undefined || (Array.isArray(value.revoked_accounts) && value.revoked_accounts.every(text))) &&
    (value.machine_browser === undefined || value.machine_browser === "enabled" || value.machine_browser === "not_enabled") &&
    (value.legacy_browsers === undefined || (Array.isArray(value.legacy_browsers) && value.legacy_browsers.every(validLegacyBrowser))) &&
    (value.browser_shares === undefined || (Array.isArray(value.browser_shares) && value.browser_shares.every(validBrowserShare))) &&
    (value.browser_share_families === undefined || (record(value.browser_share_families) && Object.values(value.browser_share_families).every(text))) &&
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
          (value.registry.revision >= old.registry.revision && (value.display_preferences?.revision ?? 0) >= (old.display_preferences?.revision ?? 0))
        ) { snapshot = { data: value, error: "" }; snapshotOwner = owner; }
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
