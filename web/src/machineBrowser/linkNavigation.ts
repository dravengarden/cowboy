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

/** Whether a share-capable Controller (`browser_identity` reported, or an
 * identity-bound caller) lacks server-side `expected_browser` support on
 * `/browser/navigate`. Opening the viewer would fail with HTTP 422, so callers
 * refuse beforehand with an upgrade-required message. */
export function linkRequiresControllerUpgrade(
  read: BrowserRead | undefined,
  expectedBrowser?: string,
): boolean {
  if (!read) return false;
  const identityAware = read.browser_identity !== undefined || expectedBrowser !== undefined;
  return identityAware && read.navigate_identity_fenced !== true;
}

/** A pending link (pinned sign-in or unpinned page shortcut) stays active
 * only while the sheet remains on its original browser binding (`epoch === 0`)
 * and matches `expectedBrowser` when captured. Any rebind cancels it so it
 * never opens or navigates a different browser profile. */
export function activeBrowserLink(
  link: BrowserLink | undefined,
  binding: IdentityBinding,
): BrowserLink | undefined {
  if (!link) return undefined;
  if (binding.epoch > 0) return undefined;
  if (
    link.expectedBrowser !== undefined &&
    binding.identity !== undefined &&
    binding.identity !== link.expectedBrowser
  ) {
    return undefined;
  }
  return link;
}

/** Whether a reconnect step must be suppressed because the latest read names
 * a different browser identity than this panel's bound identity (before React
 * rebinds the panel). */
export function resumeBlockedByIdentity(
  boundIdentity: string | undefined,
  observedIdentity: string | undefined,
): boolean {
  return (
    boundIdentity !== undefined &&
    observedIdentity !== undefined &&
    observedIdentity !== boundIdentity
  );
}

/** How a panel asked to show a link enters the browser: when pinned to
 * `machine`, the browser on that Machine as `autoEntry` would (anything else
 * through a pinned open); when unpinned, the account's own browser once a
 * successful read confirms placement (leaving first-open Machine choice and
 * another device's attached display to the owner). Never acts when the
 * Controller lacks identity-fenced navigation or the read already names a
 * different browser identity than expected. */
export function linkEntry(
  read: BrowserRead | undefined,
  machine?: string,
  expectedBrowser?: string,
): "open" | "attach" | undefined {
  if (linkRequiresControllerUpgrade(read, expectedBrowser)) {
    return undefined;
  }
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
 * Passing `attachmentId` and `expectedBrowser` to `navigate` fences both the
 * Machine attachment and the Service share membership. */
export function linkTarget(
  model: BrowserModel,
  machine?: string,
  expectedBrowser?: string,
): ConfirmedLinkTarget | undefined {
  if (linkRequiresControllerUpgrade(model.read, expectedBrowser)) {
    return undefined;
  }
  if (model.read?.navigate_identity_fenced === true && expectedBrowser === undefined) {
    return undefined;
  }
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
