import { isImeComposing } from "../vim/imeStatusStore";
import { desktopImeKeyIsReserved } from "./imeShortcutPolicy";

/** Keep Desktop commands layout-independent without stealing native text input. */
export function desktopImeOwnsKey(event: KeyboardEvent): boolean {
  const element = event.target instanceof Element ? event.target : null;
  const normalCommandSink = element?.matches("[data-vim-command-sink]") ??
    false;
  return desktopImeKeyIsReserved(
    event,
    normalCommandSink,
    isImeComposing(),
  );
}
