/** Pure rules of owner-defined shared browser profiles: which cards may join
 * a share, why one may not, and how a shared card's Browser entry and reset
 * confirmation name the members it affects. A share holds at most one member
 * per account-usage family, so two Claude (or two OpenAI) accounts never use
 * one browser. See docs/machine-browser.md, "Shared browser profiles". */
import type {
  BrowserShare,
  BrowserShareMember,
  BrowserShareMemberRef,
  LegacyBrowser,
  SubscriptionSnapshot,
} from "../subscriptionAccounts";
import { legacySubscriptionLabel } from "../subscriptionDisplay";
import type { BrowserReadShare } from "./api";

const FAMILY_NAMES: Record<string, string> = {
  anthropic: "Claude",
  openai: "OpenAI",
  xai: "Grok",
  deepseek: "DeepSeek",
};

/** The product name of an account-usage family. */
export function familyName(family: string): string {
  return FAMILY_NAMES[family] ?? family;
}

/** A browser identity the owner may put into a share. */
export interface ShareCandidate {
  member: BrowserShareMemberRef;
  label: string;
  /** Unknown for a Provider without an account-usage family: never shared. */
  family: string | undefined;
  /** The identity's own browser id (named account id or card key). */
  browserId: string;
}

/** A legacy card's name: the owner's label for it, else its family. */
export function cardLabel(snapshot: SubscriptionSnapshot, card: LegacyBrowser): string {
  const provider = card.providers[0];
  const labels = snapshot.display_preferences?.values.legacy_labels;
  const own =
    provider && labels && Object.hasOwn(labels, provider)
      ? legacySubscriptionLabel(snapshot.display_preferences, provider)
      : undefined;
  return own ?? familyName(card.card);
}

/** Every card the owner may share: collected legacy cards and named
 * accounts that are not being revoked. */
export function shareCandidates(snapshot: SubscriptionSnapshot | undefined): ShareCandidate[] {
  if (!snapshot) return [];
  // Families come only from the Service's static table; a card or
  // Provider outside it has none and is offered disabled.
  const families = snapshot.browser_share_families?.providers ?? {};
  const familyCards = snapshot.browser_share_families?.cards ?? [];
  const cards = (snapshot.legacy_browsers ?? [])
    .filter((card) => card.available)
    .map((card) => ({
      member: { card: card.card },
      label: cardLabel(snapshot, card),
      family: familyCards.includes(card.card) ? card.card : undefined,
      browserId: card.account_id,
    }));
  const named = snapshot.registry.accounts
    .filter((account) => !account.revoking && !snapshot.revoked_accounts?.includes(account.id))
    .map((account) => {
      const family = families[account.provider_id];
      return {
        member: { account_id: account.id },
        label: family ? `${familyName(family)} · ${account.label}` : account.label,
        family,
        browserId: account.id,
      };
    });
  return [...cards, ...named];
}

/** The live share whose members include the identity `browserId`. */
export function shareOf(
  snapshot: SubscriptionSnapshot | undefined,
  browserId: string,
): BrowserShare | undefined {
  return snapshot?.browser_shares?.find(
    (share) => !share.deleting && share.members.some((member) => member.account_id === browserId),
  );
}

/** How a member is named: its card or account label, else its family. */
export function memberLabel(
  snapshot: SubscriptionSnapshot | undefined,
  member: Pick<BrowserShareMember, "family" | "account_id">,
): string {
  return (
    shareCandidates(snapshot).find((candidate) => candidate.browserId === member.account_id)
      ?.label ?? familyName(member.family)
  );
}

/** Why `candidate` cannot join `share`, or `undefined` when it can. */
export function joinBlocked(share: BrowserShare, candidate: ShareCandidate): string | undefined {
  if (share.deleting) return "This group is being deleted.";
  if (!candidate.family) return "This account has no known family, so it cannot share a browser.";
  const clash = share.members.find(
    (member) => member.family === candidate.family && member.account_id !== candidate.browserId,
  );
  if (clash) return `Another ${familyName(candidate.family)} account is already in this group.`;
  const bound = share.bound?.find(
    (row) => row.family === candidate.family && row.account_id !== candidate.browserId,
  );
  return bound
    ? `Another ${familyName(candidate.family)} account used this group's profile. Reset the profile before adding this one.`
    : undefined;
}

/** What a shared card's Browser entry and panel say about the share. */
export interface SharedBrowser {
  share_id: string;
  /** Membership revision: a change re-binds the panel. */
  revision: number;
  label: string;
  /** Every member the share's profile serves, this card included. */
  members: string[];
}

export function sharedBrowser(
  snapshot: SubscriptionSnapshot | undefined,
  browserId: string,
): SharedBrowser | undefined {
  const share = shareOf(snapshot, browserId);
  return (
    share && {
      share_id: share.share_id,
      revision: share.revision,
      label: share.label,
      members: share.members.map((member) => memberLabel(snapshot, member)),
    }
  );
}

/** The share the panel's own state read reports: the Service's answer for
 * exactly the browser a reset would delete, whichever entry opened the
 * panel. Members are named from the account inventory when it has them. */
export function sharedForRead(
  share: BrowserReadShare | null | undefined,
  snapshot: SubscriptionSnapshot | undefined,
): SharedBrowser | undefined {
  if (!share) return undefined;
  return {
    share_id: share.share_id,
    revision: share.revision,
    label: share.label,
    members: share.members.map((member) => memberLabel(snapshot, member)),
  };
}

/** What the panel says about sharing: the Service's read once it named the
 * browser identity, else (no read yet, or one synthesized without it) the
 * inventory's view, so the warning never drops before authoritative
 * metadata arrives. */
export function panelShared(
  read: { browser_identity?: string; share?: BrowserReadShare | null } | undefined,
  snapshot: SubscriptionSnapshot | undefined,
  inventory: SharedBrowser | undefined,
): SharedBrowser | undefined {
  return read?.browser_identity !== undefined ? sharedForRead(read.share, snapshot) : inventory;
}

/** Whether resets must name a browser identity: always, unless a loaded
 * inventory proves the Service has no shared profiles. While the inventory
 * is loading or failed, an identity-less reset is not offered. */
export function identityRequiredFor(snapshot: SubscriptionSnapshot | undefined): boolean {
  return !(snapshot !== undefined && snapshot.browser_shares === undefined);
}

/** The one-shot entry actions of a sheet (auto-open, the link's
 * navigation), held above its keyed panel so a re-bind never repeats them. */
export interface EntryLatch {
  autoOpened: boolean;
  linked: boolean;
}

export function newEntryLatch(): EntryLatch {
  return { autoOpened: false, linked: false };
}

/** Take a one-shot action: `true` exactly once per latch. */
export function takeLatch(latch: EntryLatch, action: keyof EntryLatch): boolean {
  if (latch[action]) return false;
  latch[action] = true;
  return true;
}

/** How the panel treats the browser identity of a new read: the first
 * one binds it, an equal one keeps it, another one re-binds the panel
 * (its model, viewer and confirmations are dropped before the new identity
 * is accepted). A read without an identity changes nothing. */
export function identityBinding(
  bound: string | undefined,
  next: string | undefined,
): "bind" | "keep" | "rebind" {
  if (next === undefined || next === bound) return "keep";
  return bound === undefined ? "bind" : "rebind";
}

/** The key that re-binds a card's browser panel: its own browser, or a
 * share at a membership revision. Moving the card between groups starts
 * the panel over (model, viewer and confirmations). */
export function browserIdentityKey(shared: SharedBrowser | undefined): string {
  return shared ? `share:${shared.share_id}:${shared.revision}` : "own";
}

/** The reset confirmation's note for a shared browser. */
export function sharedResetNote(shared: SharedBrowser | undefined): string | undefined {
  if (!shared) return undefined;
  return `This profile is shared (${shared.label}). Resetting it signs out every member: ${shared.members.join(", ")}.`;
}
