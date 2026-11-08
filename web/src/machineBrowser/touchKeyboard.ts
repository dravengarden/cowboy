/** Translation of on-screen keyboard input into X11 keysyms for noVNC's
 * `sendKey`. Touch surfaces raise no key events for most characters, so the
 * viewer reads them from a hidden input's `beforeinput` and composition
 * events and forwards them one keysym at a time. */

export interface KeyAction {
  keysym: number;
  /** A physical key code when the key has one; null for typed characters. */
  code: string | null;
}

export const KEYSYM = {
  BackSpace: 0xff08,
  Tab: 0xff09,
  Return: 0xff0d,
  Escape: 0xff1b,
  Home: 0xff50,
  Left: 0xff51,
  Up: 0xff52,
  Right: 0xff53,
  Down: 0xff54,
  End: 0xff57,
  Delete: 0xffff,
} as const;

const NAMED_KEYS = {
  Enter: { keysym: KEYSYM.Return, code: "Enter" },
  Backspace: { keysym: KEYSYM.BackSpace, code: "Backspace" },
  Tab: { keysym: KEYSYM.Tab, code: "Tab" },
  Escape: { keysym: KEYSYM.Escape, code: "Escape" },
  ArrowLeft: { keysym: KEYSYM.Left, code: "ArrowLeft" },
  ArrowUp: { keysym: KEYSYM.Up, code: "ArrowUp" },
  ArrowRight: { keysym: KEYSYM.Right, code: "ArrowRight" },
  ArrowDown: { keysym: KEYSYM.Down, code: "ArrowDown" },
  Delete: { keysym: KEYSYM.Delete, code: "Delete" },
  Home: { keysym: KEYSYM.Home, code: "Home" },
  End: { keysym: KEYSYM.End, code: "End" },
} satisfies Record<string, KeyAction>;

/** X11 keysym for one Unicode scalar: Latin-1 maps directly, everything else
 * uses the `0x01000000 | code point` range. Control characters have none. */
export function keysymForCharacter(character: string): number | undefined {
  const point = character.codePointAt(0);
  if (point === undefined) return undefined;
  if (point < 0x20 || (point >= 0x7f && point < 0xa0)) return undefined;
  if (point <= 0xff) return point;
  return 0x01000000 | point;
}

/** Typed text as key presses; line breaks become Return. */
export function keyActionsForText(text: string): KeyAction[] {
  const actions: KeyAction[] = [];
  for (const character of text) {
    if (character === "\n" || character === "\r") {
      actions.push(NAMED_KEYS.Enter);
      continue;
    }
    const keysym = keysymForCharacter(character);
    if (keysym !== undefined) actions.push({ keysym, code: null });
  }
  return actions;
}

/** A key the hidden input receives as `keydown`. Backspace is deliberately
 * absent: soft keyboards report it through `beforeinput`
 * (`deleteContentBackward`) and iOS raises no `keydown` for it on an empty
 * field, so handling both would send it twice. */
export function keyActionForKey(key: string): KeyAction | undefined {
  if (key === "Backspace" || !Object.hasOwn(NAMED_KEYS, key)) return undefined;
  return (NAMED_KEYS as Record<string, KeyAction>)[key];
}

/** The keys an `InputEvent` on the hidden input stands for, or `undefined`
 * when the event belongs to an IME composition still in progress and must be
 * left to the browser until `compositionend`. */
export function keyActionsForInput(
  inputType: string,
  data: string | null,
): KeyAction[] | undefined {
  switch (inputType) {
    case "insertText":
    case "insertFromPaste":
    case "insertReplacementText":
      return keyActionsForText(data ?? "");
    case "insertLineBreak":
    case "insertParagraph":
      return [NAMED_KEYS.Enter];
    case "deleteContentBackward":
    case "deleteWordBackward":
    case "deleteSoftLineBackward":
    case "deleteHardLineBackward":
      return [NAMED_KEYS.Backspace];
    case "deleteContentForward":
    case "deleteWordForward":
      return [NAMED_KEYS.Delete];
    case "insertCompositionText":
    case "deleteCompositionText":
      return undefined;
    default:
      return [];
  }
}
