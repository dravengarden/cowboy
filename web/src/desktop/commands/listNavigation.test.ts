import { strict as assert } from "node:assert";
import { pendingItemActionKey } from "./listNavigation";

Deno.test("pending rows expose stable item-scoped action keys", () => {
  assert.equal(pendingItemActionKey("S"), "default");
  assert.equal(pendingItemActionKey("r"), "return");
  assert.equal(pendingItemActionKey("T"), "schedule");
  assert.equal(pendingItemActionKey("m"), "move");
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
