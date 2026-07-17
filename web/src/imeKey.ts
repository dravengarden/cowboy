// Some WebKit IMEs clear `isComposing` before dispatching the keydown that
// accepts a candidate. They retain the legacy 229 keyCode, so both signals are
// required to keep that key inside the native composition transaction.
export function isImeKeyEvent(
  event: Pick<KeyboardEvent, "isComposing" | "keyCode">,
): boolean {
  return event.isComposing || event.keyCode === 229;
}
