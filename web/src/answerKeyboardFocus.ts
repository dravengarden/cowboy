import { type FocusEvent, useCallback, useEffect, useState } from "react";
import { useKeyboardOpen } from "./keyboardInset";

// A question card with a free-text answer is the one Composer-stack surface
// that raises the software keyboard besides the Composer itself. With the
// keyboard up, the visible area is about half the screen, so the card, Plan,
// the Composer and the session bar can no longer share it: the card pushed
// past the top edge with its heading and answer unreachable. Like the
// Composer's own keyboard Focus Mode, the card then owns the stack alone.
//
// The overlay marks itself with this attribute; the Composer stack hides its
// other slots and the app column tucks the session bar while it is present.
export const ANSWER_KEYBOARD_ATTRIBUTE = "data-answer-keyboard";

const ANSWERING = `&:has(> [${ANSWER_KEYBOARD_ATTRIBUTE}='true'])`;

/** Stack rules while a text answer is typed on the keyboard (Mobile only).
 *  The status strip floats over the column's top edge, so the card is held
 *  clear of it instead of running under the clock. The bound is in viewport
 *  units: a percentage cannot size the footer around it (the footer would
 *  stay column-tall and leave a void), and the app column pads itself by the
 *  same `--kb-inset`. */
export const answerKeyboardStackSx = {
  [ANSWERING]: {
    maxHeight:
      "calc(100dvh - var(--kb-inset, 0px) - var(--cowboy-mobile-status-material-height, 0px) - 8px)",
  },
  [`${ANSWERING} > [data-composer-stack-slot]:not([${ANSWER_KEYBOARD_ATTRIBUTE}='true'])`]: {
    display: "none",
  },
} as const;

const TEXT_ENTRY =
  "textarea, input:not([type='radio']):not([type='checkbox']):not([type='button']):not([type='submit'])";

function isTextEntry(target: EventTarget | null): boolean {
  return target instanceof Element && target.matches(TEXT_ENTRY);
}

/** Whether the card's text answer owns the software keyboard. Focus arms it
 *  and only the keyboard closing releases it: iOS blurs the field when a
 *  button in the card is tapped, and the layout must not move under that tap
 *  before the keyboard itself has left. */
export function useAnswerKeyboardFocus(): {
  /** Spread on the overlay's root. */
  props: {
    "data-answer-keyboard": "true" | undefined;
    onFocus: (event: FocusEvent) => void;
  };
} {
  const keyboardOpen = useKeyboardOpen();
  const [armed, setArmed] = useState(false);
  useEffect(() => {
    if (!keyboardOpen) setArmed(false);
  }, [keyboardOpen]);
  const onFocus = useCallback((event: FocusEvent): void => {
    if (isTextEntry(event.target)) setArmed(true);
  }, []);
  return {
    props: { "data-answer-keyboard": armed && keyboardOpen ? "true" : undefined, onFocus },
  };
}
