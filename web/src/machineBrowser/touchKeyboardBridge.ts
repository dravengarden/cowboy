/** The neko-style keyboard overlay of touch surfaces: a transparent textarea
 * over the framebuffer that summons the OS keyboard and forwards what the OS
 * commits into it. Text is sent on `input` once no composition is running
 * and on `compositionend`, never during a composition, and the field is
 * emptied after every send so it never keeps what was typed. Editing and
 * navigation keys come from `keydown` (prevented, so the field stays empty);
 * printable keys are never read from key events (neko #724). */
import { type RefObject, useEffect, useState } from "react";
import { type KeyboardSink, deliverText, keyActionForKey } from "./touchKeyboard";

/** The part of a `<textarea>` the bridge uses. */
export interface KeyboardArea extends EventTarget {
  value: string;
}

/** An IME is processing the key (`keyCode` 229, for example Safari's late
 * Enter after a composition commits). */
const IME_KEY_CODE = 229;

/** Attach the bridge; returns its teardown. `sink` answers the session of
 * the viewer that owns `area`, or nothing once it is gone. */
export function bindTouchKeyboard(
  area: KeyboardArea,
  sink: () => KeyboardSink | undefined,
  onOpenChange: (open: boolean) => void,
): () => void {
  let composing = false;
  const flush = (): void => {
    const text = area.value;
    area.value = "";
    const current = sink();
    if (current) deliverText(current, text);
  };
  const onInput = (event: Event): void => {
    if (composing || (event as InputEvent).isComposing) return;
    flush();
  };
  const onCompositionStart = (): void => {
    composing = true;
  };
  const onCompositionEnd = (): void => {
    composing = false;
    flush();
  };
  const onKeyDown = (event: Event): void => {
    const key = event as KeyboardEvent;
    if (composing || key.isComposing || key.keyCode === IME_KEY_CODE) return;
    const action = keyActionForKey(key.key);
    if (!action) return;
    event.preventDefault();
    sink()?.sendKey(action);
  };
  // While the keyboard is up, a tap on the remote page must not move focus.
  const onFocus = (): void => {
    onOpenChange(true);
    sink()?.setFocusOnClick(false);
  };
  const onBlur = (): void => {
    composing = false;
    area.value = "";
    onOpenChange(false);
    sink()?.setFocusOnClick(true);
  };
  const listeners: [string, (event: Event) => void][] = [
    ["input", onInput],
    ["compositionstart", onCompositionStart],
    ["compositionend", onCompositionEnd],
    ["keydown", onKeyDown],
    ["focus", onFocus],
    ["blur", onBlur],
  ];
  for (const [type, listener] of listeners) area.addEventListener(type, listener);
  return () => {
    for (const [type, listener] of listeners) area.removeEventListener(type, listener);
  };
}

/** Whether the overlay keyboard is up; binds while `touch` is set. */
export function useTouchKeyboard(
  areaRef: RefObject<HTMLTextAreaElement | null>,
  sessionRef: RefObject<KeyboardSink | undefined>,
  touch: boolean,
): boolean {
  const [open, setOpen] = useState(false);
  useEffect(() => {
    const area = areaRef.current;
    if (!area || !touch) return undefined;
    return bindTouchKeyboard(area, () => sessionRef.current, setOpen);
  }, [areaRef, sessionRef, touch]);
  return open;
}
