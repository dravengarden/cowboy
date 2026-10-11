import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { EditorSelection, EditorState } from "@codemirror/state";
import {
  imageSourceRevealed,
  setVimImageMode,
  type VimImageMode,
  vimImageMode,
} from "./inlineImageSourceReveal";

const TOKEN = "![chart.png](cowboy-att:att-1)";
const DOC = `before\n${TOKEN}\nafter`;
const FROM = DOC.indexOf(TOKEN);
const TO = FROM + TOKEN.length;

// `inlineImages.ts` renders exactly this decision: source text with the
// picture below when true, the atomic thumbnail when false.
function shape(mode: VimImageMode, anchor: number, head = anchor): boolean {
  const state = EditorState.create({ doc: DOC, extensions: [vimImageMode] })
    .update({
      selection: EditorSelection.single(anchor, head),
      effects: setVimImageMode.of(mode),
    }).state;
  return imageSourceRevealed(state, FROM, TO);
}
const thumbnail = false;
const source = true;

test("an editor without the Vim field never reveals", () => {
  const state = EditorState.create({ doc: DOC, selection: { anchor: FROM } });
  assertEquals(imageSourceRevealed(state, FROM, TO), false);
});

test("without Vim the token is always an atomic thumbnail", () => {
  for (const head of [0, FROM, FROM + 5, TO, DOC.length]) {
    assertEquals(shape(null, head), thumbnail);
  }
});

test("the Vim Normal cursor on the token shows its source with the picture below", () => {
  assertEquals(shape("normal", FROM), source);
  assertEquals(shape("normal", FROM + 9), source);
  assertEquals(shape("normal", TO - 1), source);
  // The cursor on another line, or on the character after the token.
  assertEquals(shape("normal", 0), thumbnail);
  assertEquals(shape("normal", TO), thumbnail);
  assertEquals(shape("normal", TO + 1), thumbnail);
});

test("a Visual selection reaching into the token shows its source", () => {
  assertEquals(shape("normal", FROM, TO), source);
  assertEquals(shape("normal", 0, FROM + 1), source);
  assertEquals(shape("normal", DOC.length, TO - 1), source);
  assertEquals(shape("normal", 0, FROM), thumbnail);
});

test("Insert keeps the source open only for a caret already inside the token", () => {
  assertEquals(shape("insert", FROM + 3), source);
  assertEquals(shape("insert", FROM), thumbnail);
  assertEquals(shape("insert", TO), thumbnail);
  // The two-stage Backspace selection keeps its thumbnail and ring.
  assertEquals(shape("insert", FROM, TO + 1), thumbnail);
});
