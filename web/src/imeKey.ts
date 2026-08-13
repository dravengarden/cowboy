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
