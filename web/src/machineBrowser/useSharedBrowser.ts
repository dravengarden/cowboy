/** The one source of a card's sharing metadata for every Machine browser
 * opener (account cards, account settings, sign-in links): the shared
 * account inventory. See docs/machine-browser.md, "Shared browser profiles". */
import { type SubscriptionSnapshot, useSubscriptionAccounts } from "../subscriptionAccounts";
import { type SharedBrowser, sharedBrowser } from "./shareRules";

export function useSharedBrowser(account: string): {
  shared: SharedBrowser | undefined;
  snapshot: SubscriptionSnapshot | undefined;
} {
  const { data } = useSubscriptionAccounts();
  return { shared: sharedBrowser(data, account), snapshot: data };
}
