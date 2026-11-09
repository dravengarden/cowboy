import { test } from "bun:test";
import { assertEquals } from "../../../tools/test-assert.ts";
import { chordStrokes } from "./browserKeys";

test("back and forward press Alt then the arrow and release in reverse", () => {
  assertEquals(chordStrokes("back"), [
    { keysym: 0xffe9, code: "AltLeft", down: true },
    { keysym: 0xff51, code: "ArrowLeft", down: true },
    { keysym: 0xff51, code: "ArrowLeft", down: false },
    { keysym: 0xffe9, code: "AltLeft", down: false },
  ]);
  assertEquals(chordStrokes("forward"), [
    { keysym: 0xffe9, code: "AltLeft", down: true },
    { keysym: 0xff53, code: "ArrowRight", down: true },
    { keysym: 0xff53, code: "ArrowRight", down: false },
    { keysym: 0xffe9, code: "AltLeft", down: false },
  ]);
});

test("reload presses and releases F5", () => {
  assertEquals(chordStrokes("reload"), [
    { keysym: 0xffc2, code: "F5", down: true },
    { keysym: 0xffc2, code: "F5", down: false },
  ]);
});

test("every chord releases each key it pressed", () => {
  for (const chord of ["back", "forward", "reload"] as const) {
    const strokes = chordStrokes(chord);
    const held = new Set<number>();
    for (const stroke of strokes) {
      if (stroke.down) held.add(stroke.keysym);
      else held.delete(stroke.keysym);
    }
    assertEquals(held.size, 0);
  }
});
