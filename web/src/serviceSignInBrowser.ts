/** Where a Service sign-in may show its verification link. The legacy card
 * browsers that once kept it on the credential's Machine (owner decision
 * 2026-11-17) were retired with the cards, so it is the device page. */
import type { SubscriptionSnapshot } from "./subscriptionAccounts";

export type ServiceSignInBrowser =
  /** The account inventory is not known yet (or could not be read): show
   * nothing that opens the link anywhere. */
  | { kind: "unknown" }
  /** No Machine browser serves this Provider's card on this Service. */
  | { kind: "device" }
  | { kind: "machine"; owner: string; dataset: string; account: string };

export function serviceSignInBrowser(
  snapshot: SubscriptionSnapshot | undefined,
  owner: string | undefined,
  _provider: string,
): ServiceSignInBrowser {
  // The legacy default cards and their Machine browsers are retired
  // (docs/legacy-card-retirement.md): no card browser serves a Service
  // sign-in any more.
  return !snapshot || !owner ? { kind: "unknown" } : { kind: "device" };
}
