/** Where a Service (legacy default account) sign-in may show its
 * verification link. When the Service runs Machine browsers and the
 * Provider's usage card has a browser key, the link opens only in that
 * card's Machine browser on the login's Machine, never on this device
 * (owner decision 2026-11-17): every request of the Provider page and every
 * fingerprint it can read then come from the Machine that holds the
 * credential. Only a Service without that browser keeps the device page. */
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
  provider: string,
): ServiceSignInBrowser {
  if (!snapshot || !owner) return { kind: "unknown" };
  if (snapshot.machine_browser !== "enabled") return { kind: "device" };
  const card = snapshot.legacy_browsers?.find((browser) =>
    browser.available && browser.providers.includes(provider)
  );
  if (!card) return { kind: "device" };
  return {
    kind: "machine",
    owner,
    dataset: snapshot.dataset.dataset_id,
    account: card.account_id,
  };
}
