/** Pure entry, target, and identity-binding rules for opening a URL in an
 * account's Machine browser (either pinned to a sign-in Machine or unpinned
 * on the account's own placement). */
import type { BrowserRead, BrowserTarget } from "./api";
import { type BrowserModel, autoEntry, browserRunning } from "./browserModel";
import { needsMachineChoice } from "./entryRules";
import type { IdentityBinding } from "./shareRules";

export interface BrowserLink {
  url: string;
  machine?: string | undefined;
  expectedBrowser?: string | undefined;
}

export interface ConfirmedLinkTarget {
  target: BrowserTarget;
  attachmentId: string;
}

/** A pending page link stays active only while the sheet remains bound to
 * the browser identity shown when the owner clicked it. Once the binding
 * re-binds (`epoch > 0`) or mismatches `expectedBrowser`, the link is
 * cancelled so it never opens or navigates a different browser profile. */
export function activeBrowserLink(
  link: BrowserLink | undefined,
  binding: IdentityBinding,
): BrowserLink | undefined {
  if (!link) return undefined;
  if (link.expectedBrowser === undefined) return link;
  if (binding.epoch > 0) return undefined;
  if (binding.identity !== undefined && binding.identity !== link.expectedBrowser) {
    return undefined;
  }
  return link;
}

/** How a panel asked to show a link enters the browser: when pinned to
 * `machine`, the browser on that Machine as `autoEntry` would (anything else
 * through a pinned open); when unpinned, the account's own browser once a
 * successful read confirms placement (leaving first-open Machine choice and
 * another device's attached display to the owner). Never acts when the
 * Service's read already names a different browser identity than expected. */
export function linkEntry(
  read: BrowserRead | undefined,
  machine?: string,
  expectedBrowser?: string,
): "open" | "attach" | undefined {
  if (
    expectedBrowser !== undefined &&
    read?.browser_identity !== undefined &&
    read.browser_identity !== expectedBrowser
  ) {
    return undefined;
  }
  if (machine !== undefined) {
    return read?.machine_id === machine ? autoEntry(read.state?.state) : "open";
  }
  if (!read || needsMachineChoice(read)) return undefined;
  return autoEntry(read.state?.state);
}

/** The running browser a link may be shown in: only after this panel's own
 * viewer stream has confirmed its attachment (`attachmentId`), and (when
 * pinned or identity-bound) only on that exact Machine and browser identity.
 * Passing `attachmentId` to `navigate` fences the Machine against opening
 * the URL on another device if attachment raced or was superseded. */
export function linkTarget(
  model: BrowserModel,
  machine?: string,
  expectedBrowser?: string,
): ConfirmedLinkTarget | undefined {
  const viewer = model.viewer;
  if (!viewer?.attachmentId || !browserRunning(model.read?.state?.state)) {
    return undefined;
  }
  const actualIdentity = model.observedIdentity ?? model.read?.browser_identity;
  if (
    expectedBrowser !== undefined &&
    actualIdentity !== undefined &&
    actualIdentity !== expectedBrowser
  ) {
    return undefined;
  }
  const { target, attachmentId } = viewer;
  if (
    machine !== undefined &&
    (target.machine_id !== machine || model.read?.machine_id !== machine)
  ) {
    return undefined;
  }
  return { target, attachmentId };
}
