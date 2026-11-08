/** Pure rules of the Browser entries on account cards: which card offers
 * a browser, why one is unavailable, and when an open needs the owner's
 * Machine choice. See docs/machine-browser.md, "Viewer (core web)". */
import type { SubscriptionAccount, SubscriptionSnapshot } from "../subscriptionAccounts";
import { type BrowserRead, browserErrorText } from "./api";

export type AccountBrowserEntryState =
  | { kind: "none" }
  | { kind: "disabled"; account: SubscriptionAccount; reason: string; manage: boolean }
  | { kind: "ready"; account: SubscriptionAccount };

/** What a usage card with `id` offers. Only a named account of this
 * owner's registry has a browser; nothing is enabled implicitly. */
export function accountBrowserEntry(
  snapshot: SubscriptionSnapshot | undefined,
  id: string,
): AccountBrowserEntryState {
  const account = snapshot?.registry.accounts.find((candidate) => candidate.id === id);
  if (!snapshot || !account || account.revoking || snapshot.revoked_accounts?.includes(id)) {
    return { kind: "none" };
  }
  if (snapshot.machine_browser !== "enabled") {
    return {
      kind: "disabled",
      account,
      reason: browserErrorText("machine_browser_not_enabled"),
      manage: false,
    };
  }
  if (!account.enabled) {
    return { kind: "disabled", account, reason: "This account is disabled.", manage: true };
  }
  if (account.browser_enabled !== true) {
    return {
      kind: "disabled",
      account,
      reason: browserErrorText("browser_not_enabled"),
      manage: true,
    };
  }
  return { kind: "ready", account };
}

export interface MachineChoice {
  id: string;
  label: string;
}

/** Whether the next open needs the owner's Machine choice. */
export function needsMachineChoice(read: BrowserRead | undefined): boolean {
  return read?.choose_machine === true && read.admission === null && read.placements.length === 0;
}

/** The offered Machines, labelled by their display names. */
export function machineChoices(
  read: BrowserRead | undefined,
  machines: readonly { id: string; display_name?: string | undefined }[],
): MachineChoice[] {
  return (read?.capable_machines ?? []).map(({ machine_id }) => ({
    id: machine_id,
    label: machines.find((machine) => machine.id === machine_id)?.display_name || machine_id,
  }));
}
