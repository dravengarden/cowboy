import { isImeKeyEvent } from "./imeKey";
import { hasSendMod } from "./platform";

export function confirmEnterIntent(
  event: KeyboardEvent,
): "ignore" | "suppress" | "confirm" {
  if (
    event.key !== "Enter" || event.shiftKey || event.repeat ||
    isImeKeyEvent(event)
  ) return "ignore";
  return hasSendMod(event) ? "confirm" : "suppress";
}
