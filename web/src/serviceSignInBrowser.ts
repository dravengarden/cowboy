/** Where a Service (`default` account) sign-in may show its verification
 * link. A retired legacy card's sign-in (docs/legacy-card-retirement.md)
 * never opens on this device (owner decision 2026-11-17): every request of
 * the Provider page and every fingerprint it can read must come from the
 * Machine that holds the credential. A Controller from before the retirement
 * (Web deployed first, or a rollback) may still offer that sign-in and
 * advertise the card's Machine browser: the link then opens only there, and
 * without one it is not offered at all. Other Providers keep the device
 * page. */
import { legacyCardBrowsers, retiredDefaultProvider, type SubscriptionSnapshot } from "./subscriptionAccounts";

export type ServiceSignInBrowser =
  /** The account inventory is not known yet (or could not be read): show
   * nothing that opens the link anywhere. */
  | { kind: "unknown" }
  /** No Machine browser serves this Provider's card on this Service. */
  | { kind: "device" }
  | { kind: "machine"; owner: string; dataset: string; account: string }
  /** A retired card's sign-in without its Machine browser: never opened. */
  | { kind: "refused" };

/** Every Provider of a retired card family, closed: an older Controller's
 * snapshot may carry neither `retired_default_providers` nor a family table,
 * and such a sign-in must still never open on this device. */
const RETIRED_CARD_PROVIDERS = ["claude-code", "codex", "grok", "claude-deepseek", "codex-deepseek", "deepseek-harness"];

export function serviceSignInBrowser(
  snapshot: SubscriptionSnapshot | undefined,
  owner: string | undefined,
  provider: string,
): ServiceSignInBrowser {
  if (!snapshot || !owner) return { kind: "unknown" };
  const card = snapshot.machine_browser === "enabled"
    ? legacyCardBrowsers(snapshot).find((browser) => browser.available && browser.providers.includes(provider))
    : undefined;
  if (card) {
    return { kind: "machine", owner, dataset: snapshot.dataset.dataset_id, account: card.account_id };
  }
  return retiredDefaultProvider(snapshot, provider) || RETIRED_CARD_PROVIDERS.includes(provider)
    ? { kind: "refused" }
    : { kind: "device" };
}
