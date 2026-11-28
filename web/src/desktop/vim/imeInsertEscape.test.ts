import { test } from "bun:test";
import { assertEquals } from "../../../../tools/test-assert.ts";
import {
  type ImeEscapeKeyEvent,
  ImeInsertEscapeTracker,
  normalizeImeCompositionText,
  physicalAsciiLetter,
} from "./imeInsertEscape.ts";

function imeKey(code: string, overrides: Partial<ImeEscapeKeyEvent> = {}): ImeEscapeKeyEvent {
  return {
    code,
    key: "Process",
    keyCode: 229,
    isComposing: false,
    ctrlKey: false,
    metaKey: false,
    altKey: false,
    shiftKey: false,
    ...overrides,
  };
}

test("normalizeImeCompositionText strips syllable apostrophes, spaces and full-width letters", () => {
  assertEquals(normalizeImeCompositionText("j'j"), "jj");
  assertEquals(normalizeImeCompositionText("j j"), "jj");
  assertEquals(normalizeImeCompositionText("ｊ'ｊ"), "jj");
  assertEquals(normalizeImeCompositionText("j\u2019k"), "jk");
  assertEquals(normalizeImeCompositionText("解决"), "解决");
});

test("physicalAsciiLetter resolves unmodified letters and rejects chords", () => {
  assertEquals(physicalAsciiLetter(imeKey("KeyJ")), "j");
  assertEquals(physicalAsciiLetter(imeKey("KeyJ", { shiftKey: true })), "J");
  assertEquals(physicalAsciiLetter(imeKey("KeyJ", { ctrlKey: true })), null);
  assertEquals(physicalAsciiLetter(imeKey("KeyJ", { metaKey: true })), null);
  assertEquals(physicalAsciiLetter(imeKey("Backspace")), null);
});

test("fresh WeType/Pinyin jj composition escapes on second physical KeyJ", () => {
  let now = 0;
  const tracker = new ImeInsertEscapeTracker(() => now);
  assertEquals(tracker.onKeyDown(imeKey("KeyJ"), false, ["jj"], 1000), "arm");
  now = 5;
  assertEquals(tracker.onCompositionStart(), true);
  assertEquals(tracker.onCompositionUpdate("j", ["jj"], 1000), "none");
  now = 140;
  assertEquals(
    tracker.onKeyDown(imeKey("KeyJ", { isComposing: true }), true, ["jj"], 1000),
    "escape",
  );
  // Follow-up compositionupdate from the same abort must not double-trigger.
  assertEquals(tracker.onCompositionUpdate("j'j", ["jj"], 1000), "none");
  tracker.onCompositionEnd();
});

test("compositionupdate j'j escapes even without KeyJ on the second keydown (CDP/IME fallback)", () => {
  let now = 0;
  const tracker = new ImeInsertEscapeTracker(() => now);
  assertEquals(tracker.onCompositionStart(), true);
  now = 20;
  assertEquals(tracker.onCompositionUpdate("j", ["jj"], 1000), "none");
  now = 120;
  assertEquals(tracker.onCompositionUpdate("j'j", ["jj"], 1000), "escape");
});

test("full pinyin (jiejue) and mid-phrase jj (wo'j'j) never trigger escape", () => {
  let now = 0;
  const tracker = new ImeInsertEscapeTracker(() => now);
  // jiejue: j arms, i disqualifies the entire composition.
  assertEquals(tracker.onKeyDown(imeKey("KeyJ"), false, ["jj"], 1000), "arm");
  assertEquals(tracker.onCompositionStart(), true);
  now = 60;
  assertEquals(tracker.onKeyDown(imeKey("KeyI", { isComposing: true }), true, ["jj"], 1000), "none");
  assertEquals(tracker.onCompositionUpdate("ji", ["jj"], 1000), "none");
  now = 120;
  assertEquals(tracker.onKeyDown(imeKey("KeyJ", { isComposing: true }), true, ["jj"], 1000), "none");
  now = 180;
  assertEquals(tracker.onKeyDown(imeKey("KeyJ", { isComposing: true }), true, ["jj"], 1000), "none");
  tracker.onCompositionEnd();

  // Phrase starting with w followed by jj stays disqualified.
  now = 300;
  assertEquals(tracker.onKeyDown(imeKey("KeyW"), false, ["jj"], 1000), "none");
  assertEquals(tracker.onCompositionStart(), false);
  now = 350;
  assertEquals(tracker.onKeyDown(imeKey("KeyJ", { isComposing: true }), true, ["jj"], 1000), "none");
  now = 400;
  assertEquals(tracker.onKeyDown(imeKey("KeyJ", { isComposing: true }), true, ["jj"], 1000), "none");
  assertEquals(tracker.onCompositionUpdate("w'j'j", ["jj"], 1000), "none");
  tracker.onCompositionEnd();
});

test("slow pause past timeoutlen or Backspace disqualifies jj so abbreviated pinyin stays reachable", () => {
  let now = 0;
  const tracker = new ImeInsertEscapeTracker(() => now);
  assertEquals(tracker.onKeyDown(imeKey("KeyJ"), false, ["jj"], 1000), "arm");
  assertEquals(tracker.onCompositionStart(), true);
  now = 1200;
  assertEquals(
    tracker.onKeyDown(imeKey("KeyJ", { isComposing: true }), true, ["jj"], 1000),
    "none",
  );
  assertEquals(tracker.onCompositionUpdate("j'j", ["jj"], 1000), "none");
  tracker.onCompositionEnd();

  // Backspace disqualifies.
  now = 2000;
  assertEquals(tracker.onKeyDown(imeKey("KeyJ"), false, ["jj"], 1000), "arm");
  assertEquals(tracker.onCompositionStart(), true);
  now = 2050;
  assertEquals(tracker.onKeyDown(imeKey("Backspace", { isComposing: true }), true, ["jj"], 1000), "none");
  now = 2100;
  assertEquals(tracker.onKeyDown(imeKey("KeyJ", { isComposing: true }), true, ["jj"], 1000), "none");
});

test("non-IME plain English jj is left to codemirror-vim / inputVim", () => {
  const tracker = new ImeInsertEscapeTracker(() => 0);
  const plainJ = {
    code: "KeyJ",
    key: "j",
    keyCode: 74,
    isComposing: false,
    ctrlKey: false,
    metaKey: false,
    altKey: false,
    shiftKey: false,
  };
  assertEquals(tracker.onKeyDown(plainJ, false, ["jj"], 1000), "none");
  assertEquals(tracker.onKeyDown(plainJ, false, ["jj"], 1000), "none");
});

test("jk and empty sequence configs follow the user's configured escape mappings", () => {
  let now = 0;
  const tracker = new ImeInsertEscapeTracker(() => now);
  assertEquals(tracker.onKeyDown(imeKey("KeyJ"), false, ["jk"], 1000), "arm");
  assertEquals(tracker.onCompositionStart(), true);
  now = 80;
  assertEquals(
    tracker.onKeyDown(imeKey("KeyK", { isComposing: true }), true, ["jk"], 1000),
    "escape",
  );
  tracker.onCompositionEnd();

  // Esc-only config disables IME escape completely.
  assertEquals(tracker.onKeyDown(imeKey("KeyJ"), false, [], 1000), "none");
});

test("inline-preview non-ASCII pre-edit (Hanzi/Kana/Zhuyin) preserves physical KeyJ tracking", () => {
  let now = 0;
  const tracker = new ImeInsertEscapeTracker(() => now);
  assertEquals(tracker.onKeyDown(imeKey("KeyJ"), false, ["jj"], 1000), "arm");
  assertEquals(tracker.onCompositionStart(), true);
  now = 20;
  assertEquals(tracker.onCompositionUpdate("就", ["jj"], 1000), "none");
  now = 120;
  assertEquals(
    tracker.onKeyDown(imeKey("KeyJ", { isComposing: true }), true, ["jj"], 1000),
    "escape",
  );
  tracker.onCompositionEnd();

  // Typing a non-matching physical letter after inline preview still disqualifies.
  now = 300;
  assertEquals(tracker.onKeyDown(imeKey("KeyJ"), false, ["jj"], 1000), "arm");
  assertEquals(tracker.onCompositionStart(), true);
  assertEquals(tracker.onCompositionUpdate("就", ["jj"], 1000), "none");
  now = 360;
  assertEquals(tracker.onKeyDown(imeKey("KeyI", { isComposing: true }), true, ["jj"], 1000), "none");
  now = 420;
  assertEquals(tracker.onKeyDown(imeKey("KeyJ", { isComposing: true }), true, ["jj"], 1000), "none");
});

test("macOS WebKit/IMK compositionupdate-before-keydown never double-counts first lowercase j", () => {
  let now = 0;
  const tracker = new ImeInsertEscapeTracker(() => now);
  // First physical `j` delivers compositionstart + compositionupdate("j") BEFORE keydown(KeyJ, 229).
  assertEquals(tracker.onCompositionStart(), true);
  now = 2;
  assertEquals(tracker.onCompositionUpdate("j", ["jj"], 1000), "none");
  now = 4;
  assertEquals(
    tracker.onKeyDown(imeKey("KeyJ", { isComposing: true }), true, ["jj"], 1000),
    "arm",
  );
  // Continuing normal Chinese pinyin (`ji`) disqualifies and stays in Insert.
  now = 80;
  assertEquals(tracker.onCompositionUpdate("ji", ["jj"], 1000), "none");
  assertEquals(
    tracker.onKeyDown(imeKey("KeyI", { isComposing: true }), true, ["jj"], 1000),
    "none",
  );
  tracker.onCompositionEnd();

  // In a fresh composition under the same order, a real second `j` still escapes.
  now = 200;
  assertEquals(tracker.onCompositionStart(), true);
  now = 202;
  assertEquals(tracker.onCompositionUpdate("j", ["jj"], 1000), "none");
  now = 204;
  assertEquals(
    tracker.onKeyDown(imeKey("KeyJ", { isComposing: true }), true, ["jj"], 1000),
    "arm",
  );
  now = 300;
  assertEquals(
    tracker.onKeyDown(imeKey("KeyJ", { isComposing: true }), true, ["jj"], 1000),
    "escape",
  );
  tracker.onCompositionEnd();
});

test("orphaned non-composing IME KeyJ and held repeat KeyJ never false-trigger escape", () => {
  let now = 0;
  const tracker = new ImeInsertEscapeTracker(() => now);
  // Two non-composing keyCode 229 KeyJ events (idle CJK source without composition) never escape.
  assertEquals(tracker.onKeyDown(imeKey("KeyJ"), false, ["jj"], 1000), "arm");
  now = 120;
  assertEquals(tracker.onKeyDown(imeKey("KeyJ"), false, ["jj"], 1000), "arm");
  // When a composition actually starts right after, only the latest single `j` is bridged.
  now = 125;
  assertEquals(tracker.onCompositionStart(), true);
  assertEquals(tracker.onCompositionUpdate("j", ["jj"], 1000), "none");
  // Key repeat while holding `j` disqualifies instead of escaping.
  now = 350;
  assertEquals(
    tracker.onKeyDown(imeKey("KeyJ", { isComposing: true, repeat: true }), true, ["jj"], 1000),
    "none",
  );
  now = 420;
  assertEquals(
    tracker.onKeyDown(imeKey("KeyJ", { isComposing: true }), true, ["jj"], 1000),
    "none",
  );
  tracker.onCompositionEnd();
});

test("inline-preview non-ASCII arriving before first KeyJ still tracks real jj cleanly", () => {
  let now = 0;
  const tracker = new ImeInsertEscapeTracker(() => now);
  assertEquals(tracker.onCompositionStart(), true);
  now = 2;
  assertEquals(tracker.onCompositionUpdate("就", ["jj"], 1000), "none");
  now = 4;
  assertEquals(
    tracker.onKeyDown(imeKey("KeyJ", { isComposing: true }), true, ["jj"], 1000),
    "arm",
  );
  now = 120;
  assertEquals(
    tracker.onKeyDown(imeKey("KeyJ", { isComposing: true }), true, ["jj"], 1000),
    "escape",
  );
  tracker.onCompositionEnd();
});
