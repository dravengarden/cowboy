/** Pure entry and target rules for opening a URL in an account's Machine
 * browser (either pinned to a sign-in Machine or unpinned on the account's
 * own placement). */
import type { BrowserRead, BrowserTarget } from "./api";
import { type BrowserModel, autoEntry, browserRunning, shownTarget } from "./browserModel";
import { needsMachineChoice } from "./entryRules";

/** How a panel asked to show a link enters the browser: when pinned to
 * `machine`, the browser on that Machine as `autoEntry` would (anything else
 * through a pinned open); when unpinned, the account's own browser once a
 * Machine is placed (leaving first-open Machine choice and another device's
 * attached display to the owner). */
export function linkEntry(
  read: BrowserRead | undefined,
  machine?: string,
): "open" | "attach" | undefined {
  if (machine !== undefined) {
    return read?.machine_id === machine ? autoEntry(read.state?.state) : "open";
  }
  if (needsMachineChoice(read)) return undefined;
  return autoEntry(read?.state?.state);
}

/** The running browser a link may be shown in: when pinned to `machine`,
 * only while the panel's browser runs on that exact Machine; when unpinned,
 * once the browser runs and either this panel owns the display or nobody is
 * attached (never hijacking another device's live attached browser). */
export function linkTarget(model: BrowserModel, machine?: string): BrowserTarget | undefined {
  const target = shownTarget(model);
  if (!target || !browserRunning(model.read?.state?.state)) {
    return undefined;
  }
  if (machine !== undefined) {
    return target.machine_id === machine && model.read?.machine_id === machine
      ? target
      : undefined;
  }
  return model.viewer !== undefined || model.read?.state?.state === "detached"
    ? target
    : undefined;
}
