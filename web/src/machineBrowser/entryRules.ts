/** Pure rules of the Browser entries on account cards: which card offers
 * a browser, why one is unavailable, and when an open needs the owner's
 * Machine choice. See docs/machine-browser.md, "Viewer (core web)". */
import type { SubscriptionSnapshot } from "../subscriptionAccounts";
import { type BrowserRead, browserErrorText } from "./api";

/** What a Browser entry opens: a named registry account. */
export interface BrowserEntryAccount {
  id: string;
  label: string;
  browser_enabled: boolean;
}

export type AccountBrowserEntryState =
  | { kind: "none" }
  | { kind: "disabled"; account: BrowserEntryAccount; reason: string; manage: boolean }
  | { kind: "ready"; account: BrowserEntryAccount };

/** What a usage card with `id` offers: a named account of this owner's
 * registry. Nothing is enabled implicitly. */
export function accountBrowserEntry(
  snapshot: SubscriptionSnapshot | undefined,
  id: string,
  label: string = id,
): AccountBrowserEntryState {
  if (!snapshot) return { kind: "none" };
  const named = snapshot.registry.accounts.find((candidate) => candidate.id === id);
  if (!named || named.revoking || snapshot.revoked_accounts?.includes(id)) return { kind: "none" };
  const account: BrowserEntryAccount = { id: named.id, label: named.label || label, browser_enabled: named.browser_enabled === true };
  if (snapshot.machine_browser !== "enabled") {
    return {
      kind: "disabled",
      account,
      reason: browserErrorText("machine_browser_not_enabled"),
      manage: false,
    };
  }
  if (!named.enabled) {
    return { kind: "disabled", account, reason: "This account is disabled.", manage: true };
  }
  if (!account.browser_enabled) {
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

/** Whether a sign-in dialog may offer to show its verification link in the
 * account's ordinary Machine browser on `machine`, the Machine its login
 * runs on. The same admission as the account cards (named account, gate
 * on, browser enabled), that Machine able to run browsers now, and an open
 * pinned there being the account's own (its placement, or a first open the
 * owner may choose). Nothing about the login itself is consulted, and the
 * browser never feeds anything back into it. */
export function linkBrowserOffered(read: BrowserRead | undefined, machine: string): boolean {
  if (!read || read.admission !== null) return false;
  if (!(read.capable_machines ?? []).some((capable) => capable.machine_id === machine)) {
    return false;
  }
  return (
    read.choose_machine === true ||
    read.placements.length === 0 ||
    read.placements.some((placement) => placement.machine_id === machine)
  );
}
