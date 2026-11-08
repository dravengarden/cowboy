/** The hidden input that summons the OS keyboard on touch surfaces and
 * forwards what it receives to the remote browser as key presses. One
 * sentinel character stays in the field so a soft-keyboard Backspace is an
 * observable `beforeinput`; the field never keeps what was typed. */
import { type RefObject, useEffect, useState } from "react";
import { keyActionForKey, keyActionsForInput, keyActionsForText } from "./touchKeyboard";
import type { ViewerSession } from "./viewerSession";

export const KEYBOARD_SENTINEL = " ";

/** Attach the bridge; returns its teardown. */
export function bindTouchKeyboard(
  input: HTMLInputElement,
  session: () => ViewerSession | undefined,
  onOpenChange: (open: boolean) => void,
): () => void {
  const reset = (): void => {
    input.value = KEYBOARD_SENTINEL;
    input.setSelectionRange(KEYBOARD_SENTINEL.length, KEYBOARD_SENTINEL.length);
  };
  const send = (actions: readonly { keysym: number; code: string | null }[]): void => {
    const current = session();
    for (const action of actions) current?.sendKey(action);
  };
  const onBeforeInput = (event: InputEvent): void => {
    if (event.isComposing) return;
    const actions = keyActionsForInput(event.inputType, event.data);
    if (!actions) return;
    event.preventDefault();
    send(actions);
    reset();
  };
  const onKeyDown = (event: KeyboardEvent): void => {
    if (event.isComposing) return;
    const action = keyActionForKey(event.key);
    if (!action) return;
    event.preventDefault();
    send([action]);
    reset();
  };
  const onCompositionEnd = (event: CompositionEvent): void => {
    send(keyActionsForText(event.data));
    reset();
  };
  // While the keyboard is up, a tap on the remote page must not move focus.
  const onFocus = (): void => {
    onOpenChange(true);
    session()?.setFocusOnClick(false);
    reset();
  };
  const onBlur = (): void => {
    onOpenChange(false);
    session()?.setFocusOnClick(true);
  };
  input.addEventListener("beforeinput", onBeforeInput);
  input.addEventListener("keydown", onKeyDown);
  input.addEventListener("compositionend", onCompositionEnd);
  input.addEventListener("focus", onFocus);
  input.addEventListener("blur", onBlur);
  return () => {
    input.removeEventListener("beforeinput", onBeforeInput);
    input.removeEventListener("keydown", onKeyDown);
    input.removeEventListener("compositionend", onCompositionEnd);
    input.removeEventListener("focus", onFocus);
    input.removeEventListener("blur", onBlur);
  };
}

/** Whether the bridged keyboard is up; binds while `touch` is set. */
export function useTouchKeyboard(
  inputRef: RefObject<HTMLInputElement | null>,
  sessionRef: RefObject<ViewerSession | undefined>,
  touch: boolean,
): boolean {
  const [open, setOpen] = useState(false);
  useEffect(() => {
    const input = inputRef.current;
    if (!input || !touch) return undefined;
    return bindTouchKeyboard(input, () => sessionRef.current, setOpen);
  }, [inputRef, sessionRef, touch]);
  return open;
}
