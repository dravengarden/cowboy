import { strict as assert } from "node:assert";
import { listJumpIndex } from "./listNavigation";

Deno.test("list g chords support first and visible numeric slots", () => {
  assert.equal(listJumpIndex("g", 12), 0);
  assert.equal(listJumpIndex("1", 12), 0);
  assert.equal(listJumpIndex("4", 12), 3);
  assert.equal(listJumpIndex("0", 12), 9);
});

Deno.test("numeric list jumps clamp and reject unrelated keys", () => {
  assert.equal(listJumpIndex("9", 3), 2);
  assert.equal(listJumpIndex("x", 3), null);
  assert.equal(listJumpIndex("1", 0), null);
});
