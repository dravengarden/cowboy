import { strict as assert } from "node:assert";
import { listPageTarget, listScrollFor, pendingItemActionKey } from "./listNavigation";
const assertEquals = (actual: unknown, expected: unknown): void => assert.deepEqual(actual, expected);

Deno.test("pending rows expose stable item-scoped action keys", () => {
  assert.equal(pendingItemActionKey("S"), "default");
  assert.equal(pendingItemActionKey("r"), "return");
  assert.equal(pendingItemActionKey("T"), "schedule");
  assert.equal(pendingItemActionKey("m"), "move");
  assert.equal(pendingItemActionKey("D"), "document");
  assert.equal(pendingItemActionKey("x"), "remove");
  assert.equal(pendingItemActionKey("l"), null);
});

Deno.test("lists jump through transient labels, never per-row numbers", async () => {
  const provider = await Deno.readTextFile(
    new URL("./DesktopCommandProvider.tsx", import.meta.url),
  );
  assert.equal(provider.includes("/^[0-9]$/.test(key)"), false);
  assert.equal(provider.includes("/^[1-9]$/.test(key)"), false);
  assert.equal(provider.includes(`if (key === "'" && !reordering && !pinned)`), true);
  assert.equal(provider.includes("armPendingJumpChord(region.dataset.desktopRegion, items)"), true);
});

Deno.test("list paging moves the cursor by a share of the visible rows", () => {
  assertEquals(listPageTarget(0, 50, 10, "half-down"), 5);
  assertEquals(listPageTarget(5, 50, 10, "half-up"), 0);
  assertEquals(listPageTarget(3, 50, 10, "page-down"), 13);
  assertEquals(listPageTarget(48, 50, 10, "page-down"), 49);
  assertEquals(listPageTarget(0, 50, 1, "half-down"), 1);
});

Deno.test("zt / zz / zb place the row at the top, centre and bottom", () => {
  const viewport = { scrollTop: 100, height: 400 };
  const row = { top: 200, height: 40 };
  assertEquals(listScrollFor("top", row, viewport), 300);
  assertEquals(listScrollFor("center", row, viewport), 120);
  assertEquals(listScrollFor("bottom", row, viewport), 0);
});
