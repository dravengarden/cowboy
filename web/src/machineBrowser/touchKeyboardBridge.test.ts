import { test } from "bun:test";
import { assertEquals } from "../../../tools/test-assert.ts";
import { KEYSYM, type KeyAction, type KeyboardSink } from "./touchKeyboard";
import { type KeyboardArea, bindTouchKeyboard } from "./touchKeyboardBridge";

/** A textarea stand-in: the OS writes `value`, then raises the event. */
class FakeArea extends EventTarget implements KeyboardArea {
  value = "";
  /** The OS commits `value` and raises `input`. */
  input(value: string, isComposing = false, inputType = "insertText"): void {
    this.value = value;
    this.dispatchEvent(Object.assign(new Event("input"), { isComposing, inputType }));
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

function bound() {
  const area = new FakeArea();
  const sink = recordingSink();
  const open: boolean[] = [];
  const unbind = bindTouchKeyboard(
    area,
    () => sink,
    (value) => open.push(value),
  );
  return { area, sink, open, unbind };
}

const typed = (text: string): KeyAction[] =>
  [...text].map((character) => ({ keysym: character.charCodeAt(0), code: null }));

test("capitals and shifted symbols arrive as the committed characters (neko #724)", () => {
  const { area, sink } = bound();
  // iOS: Shift raises a keydown, then the OS commits the capital.
  assertEquals(area.key("Shift"), false);
  area.input("H");
  area.input("i");
  area.key("Shift");
  area.input("!");
  area.input("@");
  assertEquals(sink.keys, typed("Hi!@"));
  // The field is emptied after every send.
  assertEquals(area.value, "");
});

test("a Chinese IME composition sends only its committed text, once", () => {
  const { area, sink } = bound();
  area.compose("compositionstart");
  area.input("n", true, "insertCompositionText");
  area.input("ni", true, "insertCompositionText");
  area.input("你", true, "insertCompositionText");
  assertEquals(sink.keys, []);
  assertEquals(sink.pastes, []);
  // Chrome order: input (still composing), then compositionend.
  area.compose("compositionend");
  // Safari order: a late non-composing input after compositionend finds the
  // field already emptied and sends nothing again.
  area.input(area.value, false, "insertFromComposition");
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
  assertEquals(area.value, "");
});

test("Backspace, Enter, Tab and arrows go by keydown and keep the field empty", () => {
  const { area, sink } = bound();
  assertEquals(area.key("Backspace", { keyCode: 8 }), true);
  assertEquals(area.key("Enter", { keyCode: 13 }), true);
  assertEquals(area.key("Tab", { keyCode: 9 }), true);
  assertEquals(area.key("ArrowLeft", { keyCode: 37 }), true);
  assertEquals(area.key("a", { keyCode: 65 }), false);
  assertEquals(sink.keys, [
    { keysym: KEYSYM.BackSpace, code: "Backspace" },
    { keysym: KEYSYM.Return, code: "Enter" },
    { keysym: KEYSYM.Tab, code: "Tab" },
    { keysym: KEYSYM.Left, code: "ArrowLeft" },
  ]);
  // A line break that arrives as input instead (no keydown) is Return.
  area.input("\n", false, "insertLineBreak");
  assertEquals(sink.keys.at(-1), { keysym: KEYSYM.Return, code: "Enter" });
});

test("focus opens the keyboard and stops canvas focus; blur and teardown end it", () => {
  const { area, sink, open, unbind } = bound();
  const focus: boolean[] = [];
  sink.setFocusOnClick = (value) => focus.push(value);
  area.dispatchEvent(new Event("focus"));
  area.compose("compositionstart");
  area.input("zh", true, "insertCompositionText");
  area.dispatchEvent(new Event("blur"));
  assertEquals(open, [true, false]);
  assertEquals(focus, [false, true]);
  // An abandoned composition is dropped, never sent later.
  assertEquals(area.value, "");
  area.input("x");
  assertEquals(sink.keys, typed("x"));
  unbind();
  area.input("y");
  assertEquals(area.key("Enter", { keyCode: 13 }), false);
  assertEquals(sink.keys, typed("x"));
});
