/** Translation of on-screen keyboard input into what the remote receives.
 * Committed text is never inferred from key events (that is neko #724:
 * guacamole.js guesses characters from keydown and loses capitals, shifted
 * symbols and text after an IME composition on iOS). The keyboard overlay
 * reads the text the OS committed into it and sends it whole: short
 * printable ASCII as one keysym press per character (Xvnc adds any Shift
 * the keysym needs), anything else through the remote clipboard and Ctrl+V.
 * Only editing and navigation keys come from `keydown`. */

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

/** Typed text as key presses; line breaks become Return and tabs Tab. */
export function keyActionsForText(text: string): KeyAction[] {
  const actions: KeyAction[] = [];
  for (const character of text) {
    if (character === "\n" || character === "\r") {
      actions.push(NAMED_KEYS.Enter);
      continue;
    }
    if (character === "\t") {
      actions.push(NAMED_KEYS.Tab);
      continue;
    }
    const keysym = keysymForCharacter(character);
    if (keysym !== undefined) actions.push({ keysym, code: null });
  }
  return actions;
}

/** The editing and navigation key a `keydown` on the overlay stands for.
 * Printable keys are absent on purpose: their text arrives as `input`. */
export function keyActionForKey(key: string): KeyAction | undefined {
  if (!Object.hasOwn(NAMED_KEYS, key)) return undefined;
  return (NAMED_KEYS as Record<string, KeyAction>)[key];
}

/** Longer committed text is pasted rather than typed key by key. */
export const TYPED_TEXT_LIMIT = 32;

const TYPEABLE = /^[\x20-\x7e\n\r\t]*$/;

export type TextDelivery = { kind: "keys"; actions: KeyAction[] } | { kind: "paste"; text: string };

/** How committed text reaches the remote: printable ASCII up to
 * `TYPED_TEXT_LIMIT` characters as key presses, everything else (non-ASCII,
 * IME results, emoji, long insertions) as clipboard text plus Ctrl+V, which
 * no keyboard layout on the remote can garble. */
export function textDelivery(text: string): TextDelivery {
  if ([...text].length <= TYPED_TEXT_LIMIT && TYPEABLE.test(text)) {
    return { kind: "keys", actions: keyActionsForText(text) };
  }
  return { kind: "paste", text };
}

/** Where the keyboard overlay sends input: the session of the one viewer
 * that owns the overlay. */
export interface KeyboardSink {
  sendKey(action: KeyAction): void;
  /** Set the remote clipboard to `text` and press Ctrl+V. */
  pasteText(text: string): void;
  /** Whether a tap on the screen moves keyboard focus to it. */
  setFocusOnClick(value: boolean): void;
}

export function deliverText(sink: KeyboardSink, text: string): void {
  if (!text) return;
  const delivery = textDelivery(text.replace(/\r\n?/g, "\n"));
  if (delivery.kind === "paste") {
    sink.pasteText(delivery.text);
    return;
  }
  for (const action of delivery.actions) sink.sendKey(action);
}
