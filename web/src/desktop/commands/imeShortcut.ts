import { isImeKeyEvent } from "../../imeKey";
import { isImeComposing } from "../vim/imeStatusStore";
import { isTextEditingTarget } from "./shortcut";

/** Keep Desktop commands layout-independent without stealing an IME transaction. */
export function desktopImeOwnsKey(event: KeyboardEvent): boolean {
  const element = event.target instanceof Element ? event.target : null;
  const normalCommandSink = element?.matches("[data-vim-command-sink]") ??
    false;
  return isImeComposing() ||
    (!normalCommandSink && isTextEditingTarget(event.target) &&
      isImeKeyEvent(event));
}
