import { isImeKeyEvent } from "./imeKey";

/** Shared scoped command table: mobile hardware keyboards use the same verbs
 * without importing Desktop's leader or taking keys from another modal. */
export const DELIVERY_PICKER_COMMANDS = [
  { id: "delivery.activate", keys: ["Enter", "Mod+Enter"] },
  { id: "delivery.next", keys: ["ArrowDown", "j"] },
  { id: "delivery.previous", keys: ["ArrowUp", "k"] },
  { id: "delivery.stop", keys: ["Mod+."] },
  { id: "delivery.close", keys: ["Escape"] },
] as const;

interface PickerCommands {
  interact: () => void;
  activate: (id: string | undefined) => void;
  close: () => void;
  stop: () => void;
}
const owners = new WeakMap<HTMLElement, () => PickerCommands>();

export function registerDeliveryPickerCommands(
  root: HTMLElement,
  commands: () => PickerCommands,
): void {
  owners.set(root, commands);
}

/** Desktop's central dispatcher and mobile's local menu scope call this same
 * controller. Only the focused registered picker can own these commands. */
export function handleDeliveryPickerKey(event: KeyboardEvent): boolean {
  const focused = event.target instanceof HTMLElement ? event.target : null;
  const root = focused?.closest<HTMLElement>("[data-delivery-picker]");
  const commands = root && owners.get(root)?.();
  if (!root || !commands) return false;
  // MUI MenuItem otherwise interprets an unhandled composition Enter as click.
  if (event.key === "Enter" && isImeKeyEvent(event)) {
    event.preventDefault();
    event.stopPropagation();
    return true;
  }
  commands.interact();
  const command = deliveryPickerKey(event);
  if (!command) return false;
  event.preventDefault();
  event.stopPropagation();
  if (command === "close") commands.close();
  else if (command === "stop") commands.stop();
  else if (command === "activate") {
    commands.activate(focused?.dataset.deliveryCommand);
  } else if (command === "next" || command === "previous") {
    const rows = Array.from(
      root.querySelectorAll<HTMLElement>("[data-delivery-command]"),
    );
    const index = rows.indexOf(focused as HTMLElement);
    rows[(index + (command === "next" ? 1 : rows.length - 1)) % rows.length]
      ?.focus();
  }
  return true;
}

export function deliveryPickerKey(
  event: KeyboardEvent,
):
  | "activate"
  | "next"
  | "previous"
  | "stop"
  | "close"
  | "consume"
  | null {
  if (isImeKeyEvent(event)) return null;
  const mod = event.metaKey || event.ctrlKey;
  if (event.key === "Escape") return "close";
  if (mod && event.key === ".") return event.repeat ? "consume" : "stop";
  if (event.key === "Enter") {
    if (event.repeat || event.shiftKey) return "consume";
    if (!event.altKey) return "activate";
    return "consume";
  }
  if (mod || event.altKey || event.shiftKey) return null;
  if (event.key === "ArrowDown" || event.key === "j") return "next";
  if (event.key === "ArrowUp" || event.key === "k") return "previous";
  return null;
}
