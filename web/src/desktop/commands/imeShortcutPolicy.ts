import { isImeKeyEvent } from "../../imeKey";

export function desktopImeKeyIsReserved(
  event: Pick<KeyboardEvent, "isComposing" | "key" | "keyCode">,
  normalCommandSink: boolean,
  sharedComposition: boolean,
): boolean {
  return sharedComposition || (!normalCommandSink && isImeKeyEvent(event));
}
