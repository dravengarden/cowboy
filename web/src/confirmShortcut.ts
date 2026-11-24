import { desktopKeyIntent, type DesktopKeyIntentEvent } from "./desktop/commands/keyIntent";
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

/** Desktop-aware confirm Enter resolver: uses `desktopKeyIntent` so an idle
 *  CJK source reporting physical `Mod+Enter` as `Process`/229 still confirms,
 *  while real composition and editable IME keep their keys. */
export function desktopConfirmEnterIntent(
  event: DesktopKeyIntentEvent & Pick<KeyboardEvent, "repeat">,
  options?: { composing?: boolean },
): "ignore" | "suppress" | "confirm" {
  const resolved = desktopKeyIntent(event, options);
  if (resolved.owner === "ime") return "ignore";
  const isEnter = resolved.key === "Enter" || event.key === "Enter" ||
    event.code === "Enter" || event.code === "NumpadEnter";
  if (!isEnter || event.shiftKey || event.repeat) return "ignore";
  return hasSendMod(event as KeyboardEvent) ? "confirm" : "suppress";
}
