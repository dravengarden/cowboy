import {
  IME_COMPOSITION_END_HOLD_MS,
  imeOwnsEditable,
  isImeKeyEvent,
} from "../../imeKey";
import { isImeComposing } from "../vim/imeStatusStore";
import { isTextEditingTarget } from "./shortcut";
import { workspaceCommandKey } from "./workspaceCommandKey";

/**
 * The one ownership decision for a Desktop keydown.
 *
 * - `ime`: a composition (or its post-commit hold) owns the key. Do not act,
 *   and never `preventDefault()`: that aborts candidate confirmation.
 * - `text`: an unmodified key aimed at native text input. The field owns it;
 *   a surface may still claim structural keys such as `Escape`.
 * - `command`: a key the surface may bind. `key` is the physical identity
 *   (`j`, `G`, `1`, `[`, `Escape`, `ArrowLeft`) so a CJK input source that
 *   reports `Process`, 229 or a translated character keeps Vim positions.
 *
 * Modified chords are commands once no composition exists: an idle input
 * source may still label them Process/229. Unmodified keys in editable text
 * that carry an IME marker belong to the IME. Non-editable chrome cannot hold
 * marked text, so its keys resolve physically.
 */
export type DesktopKeyIntent =
  | { readonly owner: "ime" }
  | { readonly owner: "text"; readonly key: string }
  | {
    readonly owner: "command";
    readonly key: string;
    readonly modified: boolean;
  };

export type DesktopKeyIntentEvent =
  & Pick<
    KeyboardEvent,
    | "key"
    | "code"
    | "keyCode"
    | "isComposing"
    | "metaKey"
    | "ctrlKey"
    | "altKey"
    | "shiftKey"
    | "target"
  >;

const IME: DesktopKeyIntent = { owner: "ime" };

/** Physical key identity, extending workspace letters with the number row. */
export function physicalCommandKey(
  event: Pick<KeyboardEvent, "code" | "key" | "shiftKey">,
): string {
  const digit = /^(?:Digit|Numpad)(\d)$/.exec(event.code)?.[1];
  if (digit !== undefined && !event.shiftKey) return digit;
  return workspaceCommandKey(event);
}

export function desktopKeyIntent(
  event: DesktopKeyIntentEvent,
  { composing = desktopCompositionActive() }: { composing?: boolean } = {},
): DesktopKeyIntent {
  if (event.isComposing || composing) return IME;
  const key = physicalCommandKey(event);
  const modified = event.metaKey || event.ctrlKey || event.altKey;
  if (modified) return { owner: "command", key, modified };
  const element = event.target instanceof Element ? event.target : null;
  // The CodeMirror Normal sink and a native field in Vim Normal (inputVim)
  // are read-only command surfaces: their keys resolve physically.
  const vimSink = (element?.matches("[data-vim-command-sink]") ?? false) ||
    element?.getAttribute?.("data-vim-input-mode") === "normal";
  if (isTextEditingTarget(event.target) && !vimSink) {
    return isImeKeyEvent(event) ? IME : { owner: "text", key };
  }
  return { owner: "command", key, modified };
}

// Native inputs (MUI TextField, Autocomplete) do not report into the CM6
// composition store, so one document-level tracker covers them. It mirrors
// CM6: composition owns input from start until shortly after end, because
// macOS Pinyin may dispatch the committing keydown after compositionend.
let nativeComposing = false;
let nativeCompositionEndedAt = 0;
let trackerInstalled = false;

export function installNativeCompositionTracker(): void {
  if (trackerInstalled || typeof document === "undefined") return;
  trackerInstalled = true;
  document.addEventListener("compositionstart", () => {
    nativeComposing = true;
    nativeCompositionEndedAt = 0;
  }, true);
  document.addEventListener("compositionend", () => {
    nativeComposing = false;
    nativeCompositionEndedAt = Date.now();
  }, true);
}

export function desktopCompositionActive(now = Date.now()): boolean {
  return isImeComposing() ||
    imeOwnsEditable(
      nativeComposing,
      nativeCompositionEndedAt,
      now,
      IME_COMPOSITION_END_HOLD_MS,
    );
}
