import { strict as assert } from "node:assert";
import { listJumpIndex, pendingItemActionKey } from "./listNavigation";

Deno.test("list g chords support first and visible numeric slots", () => {
  assert.equal(listJumpIndex("g", 12), 0);
  assert.equal(listJumpIndex("1", 12), 0);
  assert.equal(listJumpIndex("4", 12), 3);
  assert.equal(listJumpIndex("0", 12), 9);
});

Deno.test("pending rows expose stable item-scoped action keys", () => {
  assert.equal(pendingItemActionKey("S"), "default");
  assert.equal(pendingItemActionKey("r"), "return");
  assert.equal(pendingItemActionKey("T"), "schedule");
  assert.equal(pendingItemActionKey("m"), "move");
  assert.equal(pendingItemActionKey("x"), "remove");
  assert.equal(pendingItemActionKey("l"), null);
});

Deno.test("numeric list jumps clamp and reject unrelated keys", () => {
  assert.equal(listJumpIndex("9", 3), 2);
  assert.equal(listJumpIndex("x", 3), null);
  assert.equal(listJumpIndex("1", 0), null);
});
