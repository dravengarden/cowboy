import { strict as assert } from "node:assert";
import {
  listJumpIndex,
  listJumpKey,
  pendingItemActionKey,
} from "./listNavigation";

Deno.test("list jump labels expose only the first ten visible slots", () => {
  assert.equal(listJumpKey(0), "1");
  assert.equal(listJumpKey(8), "9");
  assert.equal(listJumpKey(9), "0");
  assert.equal(listJumpKey(10), null);
  assert.equal(listJumpKey(-1), null);
});

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

Deno.test("numeric list jumps reject hidden and unrelated slots", () => {
  assert.equal(listJumpIndex("3", 3), 2);
  assert.equal(listJumpIndex("9", 3), null);
  assert.equal(listJumpIndex("x", 3), null);
  assert.equal(listJumpIndex("1", 0), null);
});
