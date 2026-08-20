const NATIVE_TEXT_SERVICE_KEYS = new Set([
  "AllCandidates",
  "Alphanumeric",
  "CodeInput",
  "Compose",
  "Convert",
  "Dead",
  "Eisu",
  "FinalMode",
  "GroupFirst",
  "GroupLast",
  "GroupNext",
  "GroupPrevious",
  "Hankaku",
  "HangulMode",
  "HanjaMode",
  "Hiragana",
  "HiraganaKatakana",
  "JunjaMode",
  "KanaMode",
  "KanjiMode",
  "Katakana",
  "ModeChange",
  "NextCandidate",
  "NonConvert",
  "PreviousCandidate",
  "Process",
  "Romaji",
  "SingleCandidate",
  "Unidentified",
  "Zenkaku",
  "ZenkakuHankaku",
]);

// Some WebKit IMEs clear `isComposing` before dispatching the keydown that
// accepts a candidate. They retain either the legacy 229 keyCode or a native
// text-service key such as `Process`, so all three signals belong to the IME.
export function isImeKeyEvent(
  event: Pick<KeyboardEvent, "isComposing" | "key" | "keyCode">,
): boolean {
  return event.isComposing || event.keyCode === 229 ||
    NATIVE_TEXT_SERVICE_KEYS.has(event.key);
}

const IME_INPUT_TYPES = new Set([
  "insertCompositionText",
  "insertFromComposition",
  "insertReplacementText",
  "deleteCompositionText",
  "deleteByComposition",
]);

/** beforeinput types that belong to a native IME transaction.
 * preventDefault or a CM6 dispatch here aborts iOS Pinyin candidate
 * confirmation — the candidate vanishes and composition dies. WeChat
 * IME often commits as a later insertText, so it can look fine while
 * the system keyboard fails. Obsidian never intercepts these. */
export function isImeInputType(inputType: string | undefined): boolean {
  return inputType !== undefined && IME_INPUT_TYPES.has(inputType);
}

/**
 * iOS Pinyin backspace often uses `deleteContentBackward` while
 * `isComposing` is still true — not `deleteCompositionText`. Obsidian
 * never preventDefaults or dispatches in that window. Cowboy must not
 * either: a host transaction commits the marked latin (`u o|sa`).
 */
export function isImeProtectedInput(
  event: Pick<InputEvent, "inputType" | "isComposing">,
  editorComposing = false,
): boolean {
  return event.isComposing || editorComposing || isImeInputType(event.inputType);
}

/**
 * iOS Pinyin backspace often fires compositionend and then compositionstart
 * in the next task, inserting leftover latin at the IME's stored range.
 * Host writes (`textarea.value` / setSelectionRange) in that gap move the
 * caret in front of the leftover (`调|a`). CM6/Obsidian never write the
 * editable; they set composing=-1 on end and only flush composition after
 * ~50ms if start did not arrive. Cowboy's React textarea must treat that
 * window as still IME-owned. `compositionEndedAt` is 0 when there is no hold.
 */
export const IME_COMPOSITION_END_HOLD_MS = 50;

export function imeOwnsEditable(
  composing: boolean,
  compositionEndedAt: number,
  now: number,
  holdMs = IME_COMPOSITION_END_HOLD_MS,
): boolean {
  if (composing) return true;
  if (compositionEndedAt <= 0) return false;
  return now - compositionEndedAt < holdMs;
}
