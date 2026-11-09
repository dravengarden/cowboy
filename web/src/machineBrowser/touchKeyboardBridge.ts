/** The neko-style keyboard overlay of touch surfaces: a transparent textarea
 * over the framebuffer that summons the OS keyboard and forwards what the OS
 * commits into it. Text is sent on `input` once no composition is running
 * and on `compositionend`, never during a composition, and the field is reset
 * after every send so it never keeps what was typed. Editing and navigation
 * keys come from `keydown` (prevented, so the field stays unchanged);
 * printable keys are never read from key events (neko #724).
 *
 * The idle field holds one sentinel character that is never transmitted, so
 * a soft keyboard that reports Backspace only as an edit (Android Gboard:
 * `keyCode` 229, then `deleteContentBackward`) still has something to delete;
 * removing it is one remote Backspace.
 *
 * The field is never rewritten under a live composition: iOS WebKit strands
 * the IME and loses the marked text (PITFALLS #110, `nativeComposition.ts`).
 * A blur during a composition keeps the value and the composition; WebKit
 * commits the marked text and fires `compositionend` after the blur, and that
 * commit is sent once to the session the composition began in, if it is
 * still the live one. */
import { type RefObject, useEffect, useState } from "react";
import { NATIVE_COMPOSITION_END_FALLBACK_MS } from "../composer/nativeComposition";
import { type KeyAction, type KeyboardSink, deliverText, keyActionForKey } from "./touchKeyboard";

/** The part of a `<textarea>` the bridge uses. */
export interface KeyboardArea extends EventTarget {
  value: string;
}

/** The idle field's only content: a zero-width space, never sent. */
export const KEYBOARD_SENTINEL = "\u200b";

/** An IME is processing the key (`keyCode` 229, for example Safari's late
 * Enter after a composition commits, or Gboard's Backspace). */
const IME_KEY_CODE = 229;

const BACKSPACE = keyActionForKey("Backspace") as KeyAction;
const RETURN = keyActionForKey("Enter") as KeyAction;

/** An edit that removed text before the caret (the sentinel, or nothing in
 * a field without one). */
function backwardDeletion(inputType: string | undefined, value: string): boolean {
  if (inputType) return inputType.startsWith("delete") && inputType.endsWith("Backward");
  return value === "";
}

function lineBreak(inputType: string | undefined): boolean {
  return inputType === "insertLineBreak" || inputType === "insertParagraph";
}

/** What the OS committed: the field minus the sentinel. */
function committedText(value: string): string {
  return value.startsWith(KEYBOARD_SENTINEL) ? value.slice(KEYBOARD_SENTINEL.length) : value;
}

/** Attach the bridge; returns its teardown. `sink` answers the session of
 * the viewer that owns `area`, or nothing once it is gone. */
export function bindTouchKeyboard(
  area: KeyboardArea,
  sink: () => KeyboardSink | undefined,
  onOpenChange: (open: boolean) => void,
  compositionEndFallbackMs = NATIVE_COMPOSITION_END_FALLBACK_MS,
): () => void {
  let composing = false;
  /** The session a running composition began in; its commit goes there. */
  let composingFor: KeyboardSink | undefined;
  let focused = false;
  let fallback: ReturnType<typeof setTimeout> | undefined;
  /** The editing key the last `keydown` already sent, so an edit the OS
   * still applies for it (Android ignores `preventDefault` there) is not
   * sent a second time. */
  let keySent: KeyAction | undefined;
  const reset = (): void => {
    if (area.value !== KEYBOARD_SENTINEL) area.value = KEYBOARD_SENTINEL;
  };
  const flush = (target: KeyboardSink | undefined): void => {
    const text = committedText(area.value);
    reset();
    if (target) deliverText(target, text);
  };
  const stopFallback = (): void => {
    clearTimeout(fallback);
    fallback = undefined;
  };
  const endComposition = (): void => {
    stopFallback();
    // A commit without a running composition (one the fallback gave up on
    // while the field held focus) belongs to the live session.
    const target = composing ? composingFor : sink();
    composing = false;
    composingFor = undefined;
    const current = sink();
    flush(current && current === target ? current : undefined);
  };
  const onInput = (event: Event): void => {
    if (composing || (event as InputEvent).isComposing) return;
    const { inputType } = event as InputEvent;
    const handled = keySent;
    keySent = undefined;
    if (backwardDeletion(inputType, area.value)) {
      reset();
      if (handled !== BACKSPACE) sink()?.sendKey(BACKSPACE);
      return;
    }
    if (lineBreak(inputType) && handled === RETURN) {
      reset();
      return;
    }
    flush(sink());
  };
  const onCompositionStart = (): void => {
    stopFallback();
    if (!composing) composingFor = sink();
    composing = true;
  };
  const onKeyDown = (event: Event): void => {
    const key = event as KeyboardEvent;
    keySent = undefined;
    if (composing || key.isComposing || key.keyCode === IME_KEY_CODE) return;
    const action = keyActionForKey(key.key);
    if (!action) return;
    event.preventDefault();
    keySent = action;
    sink()?.sendKey(action);
  };
  // While the keyboard is up, a tap on the remote page must not move focus.
  const onFocus = (): void => {
    focused = true;
    onOpenChange(true);
    sink()?.setFocusOnClick(false);
  };
  const onBlur = (): void => {
    focused = false;
    keySent = undefined;
    onOpenChange(false);
    sink()?.setFocusOnClick(true);
    if (!composing) {
      reset();
      return;
    }
    // Hide keyboard or More mid-composition: WebKit commits the marked text
    // and fires `compositionend` after this blur. Should that event never
    // arrive, the composition is over once the field stays unfocused; a
    // refocused field instead sends its text with the next plain input.
    stopFallback();
    fallback = setTimeout(() => {
      fallback = undefined;
      if (!composing) return;
      if (!focused) {
        endComposition();
        return;
      }
      composing = false;
      composingFor = undefined;
    }, compositionEndFallbackMs);
  };
  const listeners: [string, (event: Event) => void][] = [
    ["input", onInput],
    ["compositionstart", onCompositionStart],
    ["compositionend", endComposition],
    ["keydown", onKeyDown],
    ["focus", onFocus],
    ["blur", onBlur],
  ];
  reset();
  for (const [type, listener] of listeners) area.addEventListener(type, listener);
  return () => {
    for (const [type, listener] of listeners) area.removeEventListener(type, listener);
    // A composition the viewer did not outlive is dropped, never sent later.
    stopFallback();
    composing = false;
    composingFor = undefined;
    area.value = "";
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
