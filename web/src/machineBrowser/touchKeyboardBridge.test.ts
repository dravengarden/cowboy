import { test } from "bun:test";
import { assertEquals } from "../../../tools/test-assert.ts";
import { KEYSYM, type KeyAction, type KeyboardSink } from "./touchKeyboard";
import { KEYBOARD_SENTINEL, type KeyboardArea, bindTouchKeyboard } from "./touchKeyboardBridge";

/** A textarea stand-in: the OS writes `value`, then raises the event. */
class FakeArea extends EventTarget implements KeyboardArea {
  value = "";
  /** The OS inserts `text` at the caret (the end) and raises `input`. */
  input(text: string, isComposing = false, inputType = "insertText"): void {
    this.value += text;
    this.dispatchEvent(Object.assign(new Event("input"), { isComposing, inputType }));
  }
  /** The OS shows `marked` as the composition's text. */
  mark(marked: string, base: string): void {
    this.value = base + marked;
    this.dispatchEvent(
      Object.assign(new Event("input"), { isComposing: true, inputType: "insertCompositionText" }),
    );
  }
  /** The OS removes the character before the caret and raises `input`. */
  deleteBackward(inputType = "deleteContentBackward"): void {
    this.value = [...this.value].slice(0, -1).join("");
    this.dispatchEvent(Object.assign(new Event("input"), { isComposing: false, inputType }));
  }
  compose(event: "compositionstart" | "compositionend"): void {
    this.dispatchEvent(new Event(event));
  }
  /** Whether the bridge prevented the key's default action. */
  key(key: string, options: { keyCode?: number; isComposing?: boolean } = {}): boolean {
    const event = Object.assign(new Event("keydown", { cancelable: true }), {
      key,
      keyCode: options.keyCode ?? 0,
      isComposing: options.isComposing ?? false,
    });
    this.dispatchEvent(event);
    return event.defaultPrevented;
  }
}

function recordingSink(): KeyboardSink & { keys: KeyAction[]; pastes: string[] } {
  const keys: KeyAction[] = [];
  const pastes: string[] = [];
  return {
    keys,
    pastes,
    sendKey: (action) => keys.push(action),
    pasteText: (text) => pastes.push(text),
    setFocusOnClick: () => {},
  };
}

function bound(fallbackMs?: number) {
  const area = new FakeArea();
  const sink = recordingSink();
  // The viewer's session ref.
  let session: KeyboardSink | undefined = sink;
  const open: boolean[] = [];
  const unbind = bindTouchKeyboard(
    area,
    () => session,
    (value) => open.push(value),
    fallbackMs,
  );
  const replaceSession = (next: KeyboardSink | undefined): void => {
    session = next;
  };
  return { area, sink, open, unbind, replaceSession };
}

const S = KEYBOARD_SENTINEL;

const typed = (text: string): KeyAction[] =>
  [...text].map((character) => ({ keysym: character.charCodeAt(0), code: null }));

const BACKSPACE: KeyAction = { keysym: KEYSYM.BackSpace, code: "Backspace" };
const RETURN: KeyAction = { keysym: KEYSYM.Return, code: "Enter" };

test("capitals and shifted symbols arrive as the committed characters (neko #724)", () => {
  const { area, sink } = bound();
  // The idle field holds only the untransmitted sentinel.
  assertEquals(area.value, S);
  // iOS: Shift raises a keydown, then the OS commits the capital.
  assertEquals(area.key("Shift"), false);
  area.input("H");
  area.input("i");
  area.key("Shift");
  area.input("!");
  area.input("@");
  assertEquals(sink.keys, typed("Hi!@"));
  // The field is reset after every send.
  assertEquals(area.value, S);
});

test("a Chinese IME composition sends only its committed text, once", () => {
  const { area, sink } = bound();
  area.compose("compositionstart");
  area.mark("n", S);
  area.mark("ni", S);
  area.mark("你", S);
  assertEquals(sink.keys, []);
  assertEquals(sink.pastes, []);
  // Chrome order: input (still composing), then compositionend.
  area.compose("compositionend");
  // Safari order: a late non-composing input after compositionend finds the
  // field already reset and sends nothing again.
  area.input("", false, "insertFromComposition");
  assertEquals(sink.pastes, ["你"]);
  // Text typed after the composition is not lost.
  area.input("A");
  assertEquals(sink.keys, typed("A"));
  // A composition's Enter (IME keyCode 229, Safari's late commit key) is not
  // sent to the remote.
  assertEquals(area.key("Enter", { keyCode: 229 }), false);
  assertEquals(area.key("Enter", { isComposing: true }), false);
  assertEquals(sink.keys, typed("A"));
});

test("an autocorrect or suggestion replacement sends the replacement text once", () => {
  const { area, sink } = bound();
  // The field never holds sent text, so a replacement only ever carries new
  // text (autocorrect is off on the overlay); it is sent once, not doubled.
  area.input("teh");
  area.input("the ", false, "insertReplacementText");
  assertEquals(sink.keys, [...typed("teh"), ...typed("the ")]);
  // A long suggestion or a pasted paragraph goes through the clipboard.
  const paragraph = "The quick brown fox jumps over the lazy dog.";
  area.input(paragraph, false, "insertFromPaste");
  assertEquals(sink.pastes, [paragraph]);
  assertEquals(area.value, S);
});

test("Backspace, Enter, Tab and arrows go by keydown and keep the field unchanged", () => {
  const { area, sink } = bound();
  assertEquals(area.key("Backspace", { keyCode: 8 }), true);
  assertEquals(area.key("Enter", { keyCode: 13 }), true);
  assertEquals(area.key("Tab", { keyCode: 9 }), true);
  assertEquals(area.key("ArrowLeft", { keyCode: 37 }), true);
  assertEquals(area.key("a", { keyCode: 65 }), false);
  assertEquals(sink.keys, [
    BACKSPACE,
    RETURN,
    { keysym: KEYSYM.Tab, code: "Tab" },
    { keysym: KEYSYM.Left, code: "ArrowLeft" },
  ]);
  assertEquals(area.value, S);
  // A line break that arrives as input instead (no keydown) is Return.
  area.input("\n", false, "insertLineBreak");
  assertEquals(sink.keys.at(-1), RETURN);
});

test("Gboard's Backspace (keyCode 229, then deleteContentBackward) erases one remote character", () => {
  const { area, sink } = bound();
  area.input("ab");
  assertEquals(sink.keys, typed("ab"));
  // Android reports the key as unidentified and only applies the edit,
  // which removes the sentinel; that is exactly one remote Backspace.
  assertEquals(area.key("Unidentified", { keyCode: 229 }), false);
  area.deleteBackward();
  assertEquals(sink.keys, [...typed("ab"), BACKSPACE]);
  assertEquals(area.value, S);
  // Each further press erases one more, as often as the user presses.
  area.key("Unidentified", { keyCode: 229 });
  area.deleteBackward();
  assertEquals(sink.keys, [...typed("ab"), BACKSPACE, BACKSPACE]);
  // A word deletion of the sentinel is also one remote Backspace.
  area.deleteBackward("deleteWordBackward");
  assertEquals(sink.keys.length, 5);
  assertEquals(sink.keys.at(-1), BACKSPACE);
  assertEquals(area.value, S);
});

test("a Backspace or Enter keydown whose edit the OS still applies is sent once", () => {
  const { area, sink } = bound();
  // Desktop and iOS: the prevented keydown is the whole key.
  assertEquals(area.key("Backspace", { keyCode: 8 }), true);
  assertEquals(sink.keys, [BACKSPACE]);
  // A keyboard that ignores `preventDefault` still deletes the sentinel.
  area.key("Backspace", { keyCode: 8 });
  area.deleteBackward();
  assertEquals(sink.keys, [BACKSPACE, BACKSPACE]);
  area.key("Enter", { keyCode: 13 });
  area.input("\n", false, "insertLineBreak");
  assertEquals(sink.keys, [BACKSPACE, BACKSPACE, RETURN]);
  assertEquals(area.value, S);
  // The suppression covers only the edit of that key.
  area.key("Unidentified", { keyCode: 229 });
  area.deleteBackward();
  assertEquals(sink.keys, [BACKSPACE, BACKSPACE, RETURN, BACKSPACE]);
});

test("Backspace inside a composition stays with the IME", () => {
  const { area, sink } = bound();
  area.compose("compositionstart");
  area.mark("ni", S);
  area.key("Backspace", { keyCode: 229, isComposing: true });
  area.mark("n", S);
  assertEquals(sink.keys, []);
  area.compose("compositionend");
  assertEquals(sink.keys, typed("n"));
});

test("focus opens the keyboard and stops canvas focus; blur ends it", () => {
  const { area, sink, open } = bound();
  const focus: boolean[] = [];
  sink.setFocusOnClick = (value) => focus.push(value);
  area.dispatchEvent(new Event("focus"));
  area.dispatchEvent(new Event("blur"));
  assertEquals(open, [true, false]);
  assertEquals(focus, [false, true]);
  area.input("x");
  assertEquals(sink.keys, typed("x"));
});

test("iOS order: Hide keyboard mid-composition keeps the marked text, then sends the commit once", () => {
  const { area, sink } = bound();
  area.dispatchEvent(new Event("focus"));
  area.compose("compositionstart");
  area.mark("nihao", S);
  // The blur comes first; the field must not be rewritten under the IME.
  area.dispatchEvent(new Event("blur"));
  assertEquals(area.value, `${S}nihao`);
  assertEquals(sink.keys, []);
  // WebKit commits the marked text after the blur, refocusing transiently.
  area.dispatchEvent(new Event("focus"));
  area.mark("你好", S);
  area.compose("compositionend");
  area.dispatchEvent(new Event("blur"));
  area.input("", false, "insertFromComposition");
  assertEquals(sink.pastes, ["你好"]);
  assertEquals(sink.keys, []);
  assertEquals(area.value, S);
});

test("Chrome order: compositionend before the blur sends the commit once", () => {
  const { area, sink } = bound();
  area.dispatchEvent(new Event("focus"));
  area.compose("compositionstart");
  area.mark("你", S);
  area.compose("compositionend");
  area.dispatchEvent(new Event("blur"));
  assertEquals(sink.pastes, ["你"]);
  assertEquals(area.value, S);
});

test("a composition commit goes only to the session it began in, while it is live", () => {
  const { area, sink, replaceSession } = bound();
  area.dispatchEvent(new Event("focus"));
  area.compose("compositionstart");
  area.mark("ni", S);
  area.dispatchEvent(new Event("blur"));
  // The viewer's session was replaced before the commit arrived.
  const other = recordingSink();
  replaceSession(other);
  area.mark("你", S);
  area.compose("compositionend");
  assertEquals(sink.pastes, []);
  assertEquals(other.pastes, []);
  assertEquals(other.keys, []);
  assertEquals(area.value, S);
  // Later typing reaches the live session.
  area.input("k");
  assertEquals(other.keys, typed("k"));
});

test("without a compositionend the unfocused field sends the commit after the fallback", async () => {
  const { area, sink } = bound(5);
  area.dispatchEvent(new Event("focus"));
  area.compose("compositionstart");
  area.mark("zh", S);
  area.dispatchEvent(new Event("blur"));
  assertEquals(area.value, `${S}zh`);
  await new Promise((resolve) => setTimeout(resolve, 20));
  assertEquals(sink.keys, typed("zh"));
  assertEquals(area.value, S);
});

test("teardown during a composition drops it", async () => {
  const { area, sink, unbind, replaceSession } = bound(5);
  area.dispatchEvent(new Event("focus"));
  area.compose("compositionstart");
  area.mark("zh", S);
  area.dispatchEvent(new Event("blur"));
  replaceSession(undefined);
  unbind();
  assertEquals(area.value, "");
  area.mark("中", "");
  area.compose("compositionend");
  await new Promise((resolve) => setTimeout(resolve, 20));
  area.input("y");
  assertEquals(area.key("Enter", { keyCode: 13 }), false);
  assertEquals(sink.keys, []);
  assertEquals(sink.pastes, []);
});
