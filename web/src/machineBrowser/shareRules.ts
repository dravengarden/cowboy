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
  const families = snapshot.browser_share_families ?? {};
  const cards = (snapshot.legacy_browsers ?? [])
    .filter((card) => card.available)
    .map((card) => ({
      member: { card: card.card },
      label: cardLabel(snapshot, card),
      family: card.card,
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
  member: BrowserShareMember,
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
  return clash
    ? `Another ${familyName(candidate.family)} account is already in this group.`
    : undefined;
}

/** What a shared card's Browser entry and panel say about the share. */
export interface SharedBrowser {
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
      label: share.label,
      members: share.members.map((member) => memberLabel(snapshot, member)),
    }
  );
}

/** The reset confirmation's note for a shared browser. */
export function sharedResetNote(shared: SharedBrowser | undefined): string | undefined {
  if (!shared) return undefined;
  return `This profile is shared (${shared.label}). Resetting it signs out every member: ${shared.members.join(", ")}.`;
}
