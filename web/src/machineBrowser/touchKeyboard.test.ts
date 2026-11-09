import { test } from "bun:test";
import { assertEquals } from "../../../tools/test-assert.ts";
import {
  KEYSYM,
  type KeyAction,
  type KeyboardSink,
  TYPED_TEXT_LIMIT,
  deliverText,
  keyActionForKey,
  keyActionsForText,
  keysymForCharacter,
  textDelivery,
} from "./touchKeyboard";

test("characters map to Latin-1 or Unicode keysyms and control characters to none", () => {
  assertEquals(keysymForCharacter("a"), 0x61);
  assertEquals(keysymForCharacter(" "), 0x20);
  assertEquals(keysymForCharacter("é"), 0xe9);
  assertEquals(keysymForCharacter("界"), 0x01000000 | 0x754c);
  assertEquals(keysymForCharacter("😀"), 0x01000000 | 0x1f600);
  assertEquals(keysymForCharacter("\u0007"), undefined);
  assertEquals(keysymForCharacter("\u007f"), undefined);
  assertEquals(keysymForCharacter(""), undefined);
});

test("typed text becomes one press per code point with Return for line breaks", () => {
  assertEquals(keyActionsForText("ab\n界"), [
    { keysym: 0x61, code: null },
    { keysym: 0x62, code: null },
    { keysym: KEYSYM.Return, code: "Enter" },
    { keysym: 0x01000000 | 0x754c, code: null },
  ]);
  assertEquals(keyActionsForText("\r\n"), [
    { keysym: KEYSYM.Return, code: "Enter" },
    { keysym: KEYSYM.Return, code: "Enter" },
  ]);
});

test("keydown handles editing and navigation keys and never printable ones", () => {
  assertEquals(keyActionForKey("Enter"), { keysym: KEYSYM.Return, code: "Enter" });
  assertEquals(keyActionForKey("Backspace"), { keysym: KEYSYM.BackSpace, code: "Backspace" });
  assertEquals(keyActionForKey("ArrowLeft"), { keysym: KEYSYM.Left, code: "ArrowLeft" });
  assertEquals(keyActionForKey("Tab"), { keysym: KEYSYM.Tab, code: "Tab" });
  // Printable keys and Shift come as committed text, never as key guesses.
  assertEquals(keyActionForKey("a"), undefined);
  assertEquals(keyActionForKey("A"), undefined);
  assertEquals(keyActionForKey("Shift"), undefined);
  assertEquals(keyActionForKey("Unidentified"), undefined);
  assertEquals(keyActionForKey("constructor"), undefined);
});

test("short printable ASCII is typed, including capitals and shifted symbols", () => {
  assertEquals(textDelivery("Hi!"), {
    kind: "keys",
    actions: [
      { keysym: 0x48, code: null },
      { keysym: 0x69, code: null },
      { keysym: 0x21, code: null },
    ],
  });
  assertEquals(textDelivery('@#$%^&*()_+{}|:"<>?~'), {
    kind: "keys",
    actions: [...'@#$%^&*()_+{}|:"<>?~'].map((character) => ({
      keysym: character.charCodeAt(0),
      code: null,
    })),
  });
  assertEquals(textDelivery("a\tb").kind, "keys");
  assertEquals(textDelivery("x".repeat(TYPED_TEXT_LIMIT)).kind, "keys");
});

test("non-ASCII and long text are pasted whole", () => {
  assertEquals(textDelivery("中文"), { kind: "paste", text: "中文" });
  assertEquals(textDelivery("café"), { kind: "paste", text: "café" });
  assertEquals(textDelivery("👍"), { kind: "paste", text: "👍" });
  const long = "x".repeat(TYPED_TEXT_LIMIT + 1);
  assertEquals(textDelivery(long), { kind: "paste", text: long });
});

function recordingSink(): KeyboardSink & {
  keys: KeyAction[];
  pastes: string[];
  focusOnClick: boolean[];
} {
  const keys: KeyAction[] = [];
  const pastes: string[] = [];
  const focusOnClick: boolean[] = [];
  return {
    keys,
    pastes,
    focusOnClick,
    sendKey: (action) => keys.push(action),
    pasteText: (text) => pastes.push(text),
    setFocusOnClick: (value) => focusOnClick.push(value),
  };
}

test("delivered text reaches the sink once by the chosen path", () => {
  const sink = recordingSink();
  deliverText(sink, "");
  deliverText(sink, "ok\r\n");
  deliverText(sink, "你好");
  assertEquals(sink.keys, [
    { keysym: 0x6f, code: null },
    { keysym: 0x6b, code: null },
    { keysym: KEYSYM.Return, code: "Enter" },
  ]);
  assertEquals(sink.pastes, ["你好"]);
});
