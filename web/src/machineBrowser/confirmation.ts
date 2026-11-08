/** Confirmed lifecycle commands bind to the browser shown when the owner
 * opened the confirmation, never to whatever the panel shows when the owner
 * confirms: polling may switch the panel to another Machine's browser in
 * between, and a reset there would delete the wrong profile. */
import type { BrowserTarget } from "./api";
import { type BrowserModel, resetTarget, sameTarget, shownTarget } from "./browserModel";

export type ConfirmKind = "takeover" | "hibernate" | "reset" | "restart";

export interface ResetTarget {
  machine_id: string;
  revision: number;
}

export type Confirmation =
  | { kind: "reset"; target: ResetTarget }
  | { kind: "takeover" | "hibernate" | "restart"; target: BrowserTarget };

/** What a confirmation of `kind` would act on now, or `undefined` when the
 * panel shows nothing it applies to. */
export function confirmationFor(kind: ConfirmKind, model: BrowserModel): Confirmation | undefined {
  if (kind === "reset") {
    const target = resetTarget(model.read);
    return target && { kind, target };
  }
  const target = shownTarget(model);
  return target && { kind, target };
}

/** Whether an open confirmation still describes what the panel shows; once
 * it does not, the confirmation closes without acting. */
export function confirmationCurrent(confirmation: Confirmation, model: BrowserModel): boolean {
  const now = confirmationFor(confirmation.kind, model);
  if (!now) return false;
  if (confirmation.kind === "reset") {
    return (
      now.kind === "reset" &&
      now.target.machine_id === confirmation.target.machine_id &&
      now.target.revision === confirmation.target.revision
    );
  }
  return now.kind !== "reset" && sameTarget(confirmation.target, now.target);
}
